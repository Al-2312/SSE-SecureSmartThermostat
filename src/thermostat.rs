// Live thermostat state and the simulation loop that drives it.
//
// tick() moves the simulated house forward by however much real time has passed since the last
// tick, multiplied by the simulation speed. The CLI calls it before showing anything, so no
// background thread is needed. Everything it changes is saved in SQLite:
//   thermostat_state  one row: mode, target, active profile, indoor temp, HVAC output, clocks
//   energy_usage      HVAC runtime per simulated day (heat, cool, and total fan/blower time)

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::auth::{Role, User};
use crate::hvac::{self, Output};
use crate::profiles::{self, Mode, ProfileError};
use crate::sensors::{self, EnvOverrides};
use crate::sim::{self, Environment};

// 60 = one real second is one simulated minute; 1 = real time
pub const DEFAULT_SIM_SPEED: f64 = 60.0;
pub const MAX_SIM_SPEED: f64 = 3600.0;
pub const DEFAULT_ENV_FILE: &str = "sim/environment.txt";

// The simulation advances in slices this long so the HVAC can switch off part-way through a gap
const SLICE_SECS: f64 = 10.0;
// If the program was closed for a long time, simulate at most this much when it comes back
const MAX_CATCH_UP_SECS: f64 = 7.0 * 86_400.0;
const DAY_MS: i64 = 86_400_000;

pub struct SimConfig {
    pub speed: f64,
    pub env_path: PathBuf,
}

impl SimConfig {
    // Reads SIM_SPEED (1 to 3600, default 60) and SIM_ENV_FILE (default sim/environment.txt)
    pub fn from_env() -> Result<Self, String> {
        let speed = match std::env::var("SIM_SPEED") {
            Ok(raw) => parse_speed(&raw)?,
            Err(_) => DEFAULT_SIM_SPEED,
        };
        let env_path = std::env::var("SIM_ENV_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_ENV_FILE));
        Ok(SimConfig { speed, env_path })
    }
}

pub fn parse_speed(raw: &str) -> Result<f64, String> {
    match raw.trim().parse::<f64>() {
        Ok(s) if s.is_finite() && (1.0..=MAX_SIM_SPEED).contains(&s) => Ok(s),
        _ => Err(format!("SIM_SPEED must be a number from 1 (real time) to {MAX_SIM_SPEED}")),
    }
}

#[derive(Debug)]
pub enum ThermostatError {
    NotAllowed,
    AccountInactive, // deleted or locked since login
    AccessExpired,   // technician window closed, revoked, or never opened
    CorruptData,
    Profile(ProfileError),
    DatabaseError(rusqlite::Error),
}

impl From<rusqlite::Error> for ThermostatError {
    fn from(err: rusqlite::Error) -> Self {
        ThermostatError::DatabaseError(err)
    }
}

impl From<ProfileError> for ThermostatError {
    fn from(err: ProfileError) -> Self {
        ThermostatError::Profile(err)
    }
}

// Everything a status screen needs, as of the tick that produced it
#[derive(Debug)]
pub struct Status {
    pub sim_time: DateTime<Utc>,
    pub mode: Mode,
    pub target_f: i32,
    pub active_profile: Option<String>,
    pub indoor_f: f64,
    pub output: Output,
    pub environment: Environment,
    pub sensor_ok: bool,
    pub sensor_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DayUsage {
    pub day: String,
    pub heat_secs: f64,
    pub cool_secs: f64,
    pub fan_secs: f64, // all blower time: heating + cooling + fan-only
}

#[derive(Debug)]
pub struct EnergyReport {
    pub days: Vec<DayUsage>, // newest first, up to 30
    pub total: DayUsage,
}

struct State {
    mode: Mode,
    target_f: i32,
    active_profile_id: Option<u32>,
    indoor_f: f64,
    output: Output,
    last_tick_ms: i64,
    sim_ms: i64,
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn init_thermostat(conn: &Connection) -> Result<(), ThermostatError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS thermostat_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            mode TEXT NOT NULL,
            target_f INTEGER NOT NULL,
            active_profile_id INTEGER,
            indoor_f REAL NOT NULL,
            hvac_output TEXT NOT NULL,
            last_tick_ms INTEGER NOT NULL,
            sim_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS energy_usage (
            day TEXT PRIMARY KEY,
            heat_secs REAL NOT NULL DEFAULT 0,
            cool_secs REAL NOT NULL DEFAULT 0,
            fan_secs REAL NOT NULL DEFAULT 0
        );",
    )?;

    // First run only: start in Auto at 72 F, with the simulated clock starting at the real time
    let now = now_ms();
    conn.execute(
        "INSERT OR IGNORE INTO thermostat_state
            (id, mode, target_f, active_profile_id, indoor_f, hvac_output, last_tick_ms, sim_ms)
         VALUES (1, ?1, 72, NULL, 70.0, ?2, ?3, ?3)",
        params![Mode::Auto.as_str(), Output::Idle.as_str(), now],
    )?;
    Ok(())
}

fn load_state(conn: &Connection) -> Result<State, ThermostatError> {
    let (mode, target_f, active_profile_id, indoor_f, output, last_tick_ms, sim_ms) = conn.query_row(
        "SELECT mode, target_f, active_profile_id, indoor_f, hvac_output, last_tick_ms, sim_ms
         FROM thermostat_state WHERE id = 1",
        [],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i32>(1)?,
                r.get::<_, Option<u32>>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        },
    )?;

    Ok(State {
        mode: Mode::from_str(&mode).ok_or(ThermostatError::CorruptData)?,
        target_f,
        active_profile_id,
        indoor_f,
        output: Output::from_str(&output).ok_or(ThermostatError::CorruptData)?,
        last_tick_ms,
        sim_ms,
    })
}

fn save_state(conn: &Connection, st: &State) -> Result<(), ThermostatError> {
    conn.execute(
        "UPDATE thermostat_state SET mode = ?1, target_f = ?2, active_profile_id = ?3, indoor_f = ?4,
            hvac_output = ?5, last_tick_ms = ?6, sim_ms = ?7
         WHERE id = 1",
        params![
            st.mode.as_str(),
            st.target_f,
            st.active_profile_id,
            st.indoor_f,
            st.output.as_str(),
            st.last_tick_ms,
            st.sim_ms
        ],
    )?;
    Ok(())
}

pub fn tick_now(conn: &Connection, cfg: &SimConfig) -> Result<Status, ThermostatError> {
    tick(conn, cfg, now_ms())
}

// `now_ms` is a parameter so tests can pretend time has passed
pub fn tick(conn: &Connection, cfg: &SimConfig, now_ms: i64) -> Result<Status, ThermostatError> {
    let mut st = load_state(conn)?;

    // A bad override file is a sensor fault: fall back to generated weather, stop heating/cooling
    let (overrides, sensor_ok, sensor_message) = match sensors::read_environment(&cfg.env_path) {
        Ok(Some(o)) => (o, true, None),
        Ok(None) => (EnvOverrides::default(), true, None),
        Err(e) => (EnvOverrides::default(), false, Some(e.to_string())),
    };

    // A clock that went backwards moves nothing
    let real_secs = (now_ms - st.last_tick_ms).max(0) as f64 / 1000.0;
    let mut remaining = (real_secs * cfg.speed).min(MAX_CATCH_UP_SECS);

    // simulated day number -> [heat, cool, fan] seconds
    let mut usage: BTreeMap<i64, [f64; 3]> = BTreeMap::new();

    while remaining > 0.0 {
        let dt = remaining.min(SLICE_SECS);
        let env = sim::generate(st.sim_ms).with_overrides(&overrides);

        st.output = hvac::decide(st.mode, st.target_f, st.indoor_f, st.output, sensor_ok);
        st.indoor_f = sim::step_indoor(st.indoor_f, env.outdoor_f, st.output, dt);

        let day = usage.entry(st.sim_ms.div_euclid(DAY_MS)).or_insert([0.0; 3]);
        match st.output {
            Output::Heating => {
                day[0] += dt;
                day[2] += dt;
            }
            Output::Cooling => {
                day[1] += dt;
                day[2] += dt;
            }
            Output::FanOnly => day[2] += dt,
            Output::Idle => {}
        }

        st.sim_ms += (dt * 1000.0).round() as i64;
        remaining -= dt;
    }

    // Decide once more so a setting change or sensor fault shows up immediately
    let env = sim::generate(st.sim_ms).with_overrides(&overrides);
    st.output = hvac::decide(st.mode, st.target_f, st.indoor_f, st.output, sensor_ok);
    st.last_tick_ms = now_ms;

    // State and energy totals are saved together or not at all
    let tx = conn.unchecked_transaction()?;
    save_state(&tx, &st)?;
    for (day, secs) in &usage {
        tx.execute(
            "INSERT INTO energy_usage (day, heat_secs, cool_secs, fan_secs) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(day) DO UPDATE SET
                heat_secs = heat_secs + excluded.heat_secs,
                cool_secs = cool_secs + excluded.cool_secs,
                fan_secs  = fan_secs  + excluded.fan_secs",
            params![day_label(*day), secs[0], secs[1], secs[2]],
        )?;
    }
    tx.commit()?;

    build_status(conn, &st, env, sensor_ok, sensor_message)
}

fn day_label(day: i64) -> String {
    DateTime::from_timestamp_millis(day * DAY_MS)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn build_status(
    conn: &Connection,
    st: &State,
    environment: Environment,
    sensor_ok: bool,
    sensor_message: Option<String>,
) -> Result<Status, ThermostatError> {
    let active_profile = match st.active_profile_id {
        Some(id) => conn
            .query_row("SELECT name FROM profiles WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?,
        None => None,
    };

    Ok(Status {
        sim_time: DateTime::from_timestamp_millis(st.sim_ms).ok_or(ThermostatError::CorruptData)?,
        mode: st.mode,
        target_f: st.target_f,
        active_profile,
        indoor_f: st.indoor_f,
        output: st.output,
        environment,
        sensor_ok,
        sensor_message,
    })
}

// Guests can only pick profiles; manual control and reports are for homeowners and technicians
fn require_homeowner_or_technician(user: &User) -> Result<(), ThermostatError> {
    match user.role {
        Role::Homeowner | Role::Technician => Ok(()),
        Role::Guest => Err(ThermostatError::NotAllowed),
    }
}

// Checked on every action, not just at login, and read fresh from the users table rather than the
// User copied at login, so locking, deleting or revoking an account takes effect mid-session.
// A technician needs an open window; one with no window set is refused (fail closed).
fn require_active_account(conn: &Connection, user: &User) -> Result<(), ThermostatError> {
    let row: Option<(bool, Option<DateTime<Utc>>)> = conn
        .query_row(
            "SELECT is_locked, valid_until FROM users WHERE id = ?1",
            params![user.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    let Some((is_locked, valid_until)) = row else {
        return Err(ThermostatError::AccountInactive);
    };
    if is_locked {
        return Err(ThermostatError::AccountInactive);
    }

    let now = Utc::now();
    let window_open = match user.role {
        Role::Technician => valid_until.is_some_and(|end| now < end),
        Role::Homeowner | Role::Guest => valid_until.is_none_or(|end| now < end),
    };
    if !window_open {
        return Err(ThermostatError::AccessExpired);
    }
    Ok(())
}

// Ticks first so the time before the change is simulated under the old settings, saves the
// change, then ticks again so the HVAC reacts right away
fn change_settings(
    conn: &Connection,
    cfg: &SimConfig,
    change: impl FnOnce(&mut State),
) -> Result<Status, ThermostatError> {
    tick_now(conn, cfg)?;
    let mut st = load_state(conn)?;
    change(&mut st);
    save_state(conn, &st)?;
    tick_now(conn, cfg)
}

// A manual change replaces whatever profile was active
pub fn set_mode(conn: &Connection, cfg: &SimConfig, user: &User, mode: Mode) -> Result<Status, ThermostatError> {
    require_homeowner_or_technician(user)?;
    require_active_account(conn, user)?;
    change_settings(conn, cfg, |st| {
        st.mode = mode;
        st.active_profile_id = None;
    })
}

pub fn set_target(conn: &Connection, cfg: &SimConfig, user: &User, target_f: i32) -> Result<Status, ThermostatError> {
    require_homeowner_or_technician(user)?;
    require_active_account(conn, user)?;
    profiles::check_target(target_f)?;
    change_settings(conn, cfg, |st| {
        st.target_f = target_f;
        st.active_profile_id = None;
    })
}

// Any role. Copies the profile's mode and target into the live settings; editing the profile
// later doesn't change the live settings until it is activated again.
pub fn apply_profile(conn: &Connection, cfg: &SimConfig, user: &User, profile_id: u32) -> Result<Status, ThermostatError> {
    require_active_account(conn, user)?;
    let profile = profiles::activate_profile(conn, user, profile_id)?;
    change_settings(conn, cfg, |st| {
        st.mode = profile.mode;
        st.target_f = profile.target_f;
        st.active_profile_id = Some(profile.id);
    })
}

pub fn energy_report(conn: &Connection, user: &User) -> Result<EnergyReport, ThermostatError> {
    require_homeowner_or_technician(user)?;
    require_active_account(conn, user)?;

    let mut stmt = conn.prepare(
        "SELECT day, heat_secs, cool_secs, fan_secs FROM energy_usage ORDER BY day DESC LIMIT 30",
    )?;
    let days = stmt
        .query_map([], |r| {
            Ok(DayUsage { day: r.get(0)?, heat_secs: r.get(1)?, cool_secs: r.get(2)?, fan_secs: r.get(3)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let total = conn.query_row(
        "SELECT COALESCE(SUM(heat_secs), 0), COALESCE(SUM(cool_secs), 0), COALESCE(SUM(fan_secs), 0)
         FROM energy_usage",
        [],
        |r| {
            Ok(DayUsage { day: "All time".to_string(), heat_secs: r.get(0)?, cool_secs: r.get(1)?, fan_secs: r.get(2)? })
        },
    )?;

    Ok(EnergyReport { days, total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const HOMEOWNER_ID: u32 = 1;
    const TECHNICIAN_ID: u32 = 2;
    const GUEST_ID: u32 = 3;

    // Same users table as database::init_db, with one account per role. The technician's window
    // is open for the next hour.
    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                username TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL,
                valid_until INTEGER,
                failed_attempts INTEGER DEFAULT 0,
                is_locked INTEGER DEFAULT 0
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO users (id, username, password_hash, role, valid_until) VALUES
                (1, 'owner', 'x', 'Homeowner', NULL),
                (2, 'tech', 'x', 'Technician', ?1),
                (3, 'guest', 'x', 'Guest', NULL)",
            params![Utc::now() + chrono::Duration::hours(1)],
        )
        .unwrap();
        profiles::init_profiles(&conn).unwrap();
        init_thermostat(&conn).unwrap();
        conn
    }

    // The User as it was at login. Its valid_until is deliberately not what the checks rely on.
    fn user(role: Role) -> User {
        let id = match role {
            Role::Homeowner => HOMEOWNER_ID,
            Role::Technician => TECHNICIAN_ID,
            Role::Guest => GUEST_ID,
        };
        User { id, username: "tester".to_string(), role, valid_until: None, failed_attempts: 0, is_locked: false }
    }

    fn set_tech_window(conn: &Connection, end: Option<DateTime<Utc>>) {
        conn.execute("UPDATE users SET valid_until = ?1 WHERE id = ?2", params![end, TECHNICIAN_ID]).unwrap();
    }

    fn home_id(conn: &Connection) -> u32 {
        profiles::list_profiles(conn).unwrap().into_iter().find(|p| p.name == "Home").unwrap().id
    }

    // A config whose override file contains `contents` (None = no file at all)
    fn config(name: &str, speed: f64, contents: Option<&str>) -> SimConfig {
        let env_path = std::env::temp_dir().join(format!("sse_thermostat_{}_{name}.txt", std::process::id()));
        match contents {
            Some(text) => std::fs::write(&env_path, text).unwrap(),
            None => {
                std::fs::remove_file(&env_path).ok();
            }
        }
        SimConfig { speed, env_path }
    }

    fn cleanup(cfg: &SimConfig) {
        if Path::new(&cfg.env_path).exists() {
            std::fs::remove_file(&cfg.env_path).ok();
        }
    }

    fn last_tick(conn: &Connection) -> i64 {
        load_state(conn).unwrap().last_tick_ms
    }

    #[test]
    fn heats_to_target_and_records_runtime() {
        let conn = setup();
        let cfg = config("heat", 1.0, Some("outdoor_f = 40"));
        let h = user(Role::Homeowner);
        set_mode(&conn, &cfg, &h, Mode::Heat).unwrap();
        set_target(&conn, &cfg, &h, 75).unwrap();

        let status = tick(&conn, &cfg, last_tick(&conn) + 3_600_000).unwrap(); // one hour later
        cleanup(&cfg);

        assert!((73.5..=75.5).contains(&status.indoor_f), "{}", status.indoor_f);
        let total = energy_report(&conn, &h).unwrap().total;
        assert!(total.heat_secs > 600.0);
        assert_eq!(total.cool_secs, 0.0);
        assert!(total.fan_secs >= total.heat_secs); // the blower runs whenever the heat does
    }

    #[test]
    fn house_drifts_toward_outdoor_when_off() {
        let conn = setup();
        let cfg = config("off", 1.0, Some("outdoor_f = 40"));
        let h = user(Role::Homeowner);
        set_mode(&conn, &cfg, &h, Mode::Off).unwrap();

        // The few real milliseconds before set_mode ran in the starting Auto mode, so measure
        // only the hour simulated below
        let before = energy_report(&conn, &h).unwrap().total;
        let status = tick(&conn, &cfg, last_tick(&conn) + 3_600_000).unwrap();
        cleanup(&cfg);

        assert!(status.indoor_f < 70.0 && status.indoor_f > 40.0);
        assert_eq!(status.output, Output::Idle);
        assert_eq!(energy_report(&conn, &h).unwrap().total, before);
    }

    #[test]
    fn speed_multiplies_simulated_time() {
        let conn = setup();
        let cfg = config("speed", 60.0, None);
        let before = load_state(&conn).unwrap().sim_ms;
        tick(&conn, &cfg, last_tick(&conn) + 60_000).unwrap(); // one real minute
        assert_eq!(load_state(&conn).unwrap().sim_ms - before, 3_600_000); // one simulated hour
    }

    #[test]
    fn clock_going_backwards_changes_nothing() {
        let conn = setup();
        let cfg = config("backwards", 60.0, None);
        let before = load_state(&conn).unwrap();
        tick(&conn, &cfg, before.last_tick_ms - 3_600_000).unwrap();
        let after = load_state(&conn).unwrap();
        assert_eq!(after.sim_ms, before.sim_ms);
        assert_eq!(after.indoor_f, before.indoor_f);
    }

    #[test]
    fn long_absence_is_capped() {
        let conn = setup();
        let cfg = config("cap", 1.0, None);
        let before = load_state(&conn).unwrap().sim_ms;
        tick(&conn, &cfg, last_tick(&conn) + 30 * DAY_MS).unwrap();
        assert_eq!(load_state(&conn).unwrap().sim_ms - before, (MAX_CATCH_UP_SECS * 1000.0) as i64);
    }

    #[test]
    fn bad_sensor_file_stops_heating() {
        let conn = setup();
        let cfg = config("fault", 1.0, Some("outdoor_f = banana"));
        let h = user(Role::Homeowner);
        set_mode(&conn, &cfg, &h, Mode::Heat).unwrap();
        set_target(&conn, &cfg, &h, 90).unwrap();

        let status = tick(&conn, &cfg, last_tick(&conn) + 600_000).unwrap();
        cleanup(&cfg);

        assert!(!status.sensor_ok);
        assert!(status.sensor_message.is_some());
        assert_eq!(status.output, Output::Idle);
        assert_eq!(energy_report(&conn, &h).unwrap().total.heat_secs, 0.0);
    }

    #[test]
    fn guest_can_only_choose_profiles() {
        let conn = setup();
        let cfg = config("guest", 60.0, None);
        let g = user(Role::Guest);

        assert!(matches!(set_mode(&conn, &cfg, &g, Mode::Heat), Err(ThermostatError::NotAllowed)));
        assert!(matches!(set_target(&conn, &cfg, &g, 75), Err(ThermostatError::NotAllowed)));
        assert!(matches!(energy_report(&conn, &g), Err(ThermostatError::NotAllowed)));

        let vacation = profiles::list_profiles(&conn).unwrap().into_iter().find(|p| p.name == "Vacation").unwrap();
        let status = apply_profile(&conn, &cfg, &g, vacation.id).unwrap();
        assert_eq!(status.mode, vacation.mode);
        assert_eq!(status.target_f, vacation.target_f);
        assert_eq!(status.active_profile.as_deref(), Some("Vacation"));
    }

    #[test]
    fn technician_can_control_and_see_reports() {
        let conn = setup();
        let cfg = config("tech", 60.0, None);
        let t = user(Role::Technician);
        assert_eq!(set_mode(&conn, &cfg, &t, Mode::Cool).unwrap().mode, Mode::Cool);
        assert_eq!(set_target(&conn, &cfg, &t, 70).unwrap().target_f, 70);
        assert!(energy_report(&conn, &t).is_ok());
    }

    #[test]
    fn technician_loses_control_when_window_closes_mid_session() {
        let conn = setup();
        let cfg = config("window_closed", 60.0, None);
        let t = user(Role::Technician); // logged in while the window was open
        assert!(set_mode(&conn, &cfg, &t, Mode::Cool).is_ok());

        set_tech_window(&conn, Some(Utc::now() - chrono::Duration::minutes(1))); // closed or revoked
        assert!(matches!(set_mode(&conn, &cfg, &t, Mode::Heat), Err(ThermostatError::AccessExpired)));
        assert!(matches!(set_target(&conn, &cfg, &t, 70), Err(ThermostatError::AccessExpired)));
        assert!(matches!(apply_profile(&conn, &cfg, &t, home_id(&conn)), Err(ThermostatError::AccessExpired)));
        assert!(matches!(energy_report(&conn, &t), Err(ThermostatError::AccessExpired)));
    }

    #[test]
    fn technician_without_a_window_is_refused() {
        let conn = setup();
        let cfg = config("no_window", 60.0, None);
        set_tech_window(&conn, None);
        assert!(matches!(
            set_mode(&conn, &cfg, &user(Role::Technician), Mode::Heat),
            Err(ThermostatError::AccessExpired)
        ));
    }

    #[test]
    fn locked_or_deleted_account_is_refused() {
        let conn = setup();
        let cfg = config("inactive", 60.0, None);

        conn.execute("UPDATE users SET is_locked = 1 WHERE id = ?1", params![GUEST_ID]).unwrap();
        assert!(matches!(
            apply_profile(&conn, &cfg, &user(Role::Guest), home_id(&conn)),
            Err(ThermostatError::AccountInactive)
        ));

        conn.execute("DELETE FROM users WHERE id = ?1", params![HOMEOWNER_ID]).unwrap();
        assert!(matches!(
            set_mode(&conn, &cfg, &user(Role::Homeowner), Mode::Heat),
            Err(ThermostatError::AccountInactive)
        ));
    }

    #[test]
    fn target_must_be_in_range() {
        let conn = setup();
        let cfg = config("range", 60.0, None);
        assert!(matches!(
            set_target(&conn, &cfg, &user(Role::Homeowner), 500),
            Err(ThermostatError::Profile(ProfileError::InvalidTarget))
        ));
    }

    #[test]
    fn manual_change_clears_active_profile() {
        let conn = setup();
        let cfg = config("manual", 60.0, None);
        let h = user(Role::Homeowner);
        let home = profiles::list_profiles(&conn).unwrap().into_iter().find(|p| p.name == "Home").unwrap();

        assert_eq!(apply_profile(&conn, &cfg, &h, home.id).unwrap().active_profile.as_deref(), Some("Home"));
        assert_eq!(set_target(&conn, &cfg, &h, 70).unwrap().active_profile, None);
    }

    #[test]
    fn runtime_accumulates_across_ticks() {
        let conn = setup();
        let cfg = config("accumulate", 1.0, None);
        set_mode(&conn, &cfg, &user(Role::Homeowner), Mode::Fan).unwrap();

        let start = last_tick(&conn);
        tick(&conn, &cfg, start + 60_000).unwrap();
        tick(&conn, &cfg, start + 120_000).unwrap();

        let total = energy_report(&conn, &user(Role::Homeowner)).unwrap().total;
        assert!((total.fan_secs - 120.0).abs() < 1.0, "{}", total.fan_secs);
    }

    #[test]
    fn sim_speed_must_be_in_range() {
        assert_eq!(parse_speed("60").unwrap(), 60.0);
        assert_eq!(parse_speed(" 1 ").unwrap(), 1.0);
        for bad in ["0", "0.5", "5000", "fast", "NaN", "inf", ""] {
            assert!(parse_speed(bad).is_err(), "{bad}");
        }
    }
}
