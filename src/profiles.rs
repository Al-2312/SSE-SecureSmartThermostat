// HVAC profiles: a named (mode, target temperature) pair, e.g. "Home" = Auto at 72 F.
// Design brief: four built-ins (Home, Away, Sleep, Vacation) that can be edited but not deleted.
// Any role can activate a profile; only the Homeowner can create, edit, or delete them.

use rusqlite::{Connection, OptionalExtension, params};
use crate::auth::{Role, User};

pub const MIN_TARGET_F: i32 = 30;
pub const MAX_TARGET_F: i32 = 100;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Off,
    Heat,
    Cool,
    Auto,
    Fan,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Off  => "Off",
            Mode::Heat => "Heat",
            Mode::Cool => "Cool",
            Mode::Auto => "Auto",
            Mode::Fan  => "Fan",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Off"  => Some(Mode::Off),
            "Heat" => Some(Mode::Heat),
            "Cool" => Some(Mode::Cool),
            "Auto" => Some(Mode::Auto),
            "Fan"  => Some(Mode::Fan),
            _      => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: i64,
    pub name: String,
    pub mode: Mode,
    pub target_f: i32,
    pub built_in: bool,
}

#[derive(Debug)]
pub enum ProfileError {
    NotAllowed,
    InvalidName,
    InvalidTarget,
    NotFound,
    BuiltInProtected,
    CorruptData,
    #[allow(dead_code)]
    DatabaseError(rusqlite::Error),
}

impl From<rusqlite::Error> for ProfileError {
    fn from(err: rusqlite::Error) -> Self {
        ProfileError::DatabaseError(err)
    }
}

// name, mode, target_f
const BUILT_INS: [(&str, Mode, i32); 4] = [
    ("Home",     Mode::Auto, 72),
    ("Away",     Mode::Auto, 65),
    ("Sleep",    Mode::Auto, 68),
    ("Vacation", Mode::Auto, 60), // stays above freezing-pipe territory; Off would not
];

// Names are 1 to 32 letters, digits, spaces, '-' or '_' so they can't carry control characters
pub fn check_name(name: &str) -> Result<(), ProfileError> {
    let len = name.chars().count();
    if len == 0 || len > 32 || name != name.trim() {
        return Err(ProfileError::InvalidName);
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_') {
        return Err(ProfileError::InvalidName);
    }
    Ok(())
}

pub fn check_target(target_f: i32) -> Result<(), ProfileError> {
    if !(MIN_TARGET_F..=MAX_TARGET_F).contains(&target_f) {
        return Err(ProfileError::InvalidTarget);
    }
    Ok(())
}

fn require_homeowner(user: &User) -> Result<(), ProfileError> {
    match user.role {
        Role::Homeowner => Ok(()),
        Role::Technician | Role::Guest => Err(ProfileError::NotAllowed),
    }
}

pub fn init_profiles(conn: &Connection) -> Result<(), ProfileError> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS profiles (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL COLLATE NOCASE,
            mode TEXT NOT NULL,
            target_f INTEGER NOT NULL,
            built_in INTEGER NOT NULL DEFAULT 0
        );",
        [],
    )?;

    let has_profiles: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM profiles)",
        [],
        |row| row.get(0),
    )?;

    if !has_profiles {
        for (name, mode, target_f) in BUILT_INS {
            conn.execute(
                "INSERT INTO profiles (name, mode, target_f, built_in) VALUES (?1, ?2, ?3, 1);",
                params![name, mode.as_str(), target_f],
            )?;
        }
    }

    Ok(())
}

type ProfileRow = (i64, String, String, i32, bool);

fn row_to_tuple(row: &rusqlite::Row) -> rusqlite::Result<ProfileRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
}

fn tuple_to_profile(t: ProfileRow) -> Result<Profile, ProfileError> {
    let (id, name, mode_str, target_f, built_in) = t;
    let mode = Mode::from_str(&mode_str).ok_or(ProfileError::CorruptData)?;
    Ok(Profile { id, name, mode, target_f, built_in })
}

fn get_profile(conn: &Connection, id: i64) -> Result<Profile, ProfileError> {
    let row = conn
        .query_row(
            "SELECT id, name, mode, target_f, built_in FROM profiles WHERE id = ?1",
            params![id],
            row_to_tuple,
        )
        .optional()?;

    match row {
        Some(t) => tuple_to_profile(t),
        None => Err(ProfileError::NotFound),
    }
}

// Any logged-in role may list profiles
pub fn list_profiles(conn: &Connection) -> Result<Vec<Profile>, ProfileError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, mode, target_f, built_in FROM profiles ORDER BY id",
    )?;
    let rows = stmt.query_map([], row_to_tuple)?;

    let mut profiles = Vec::new();
    for row in rows {
        profiles.push(tuple_to_profile(row?)?);
    }
    Ok(profiles)
}

pub fn create_profile(
    conn: &Connection,
    user: &User,
    name: &str,
    mode: Mode,
    target_f: i32,
) -> Result<Profile, ProfileError> {
    require_homeowner(user)?;
    check_name(name)?;
    check_target(target_f)?;

    // A duplicate name is rejected by the UNIQUE constraint on profiles.name
    conn.execute(
        "INSERT INTO profiles (name, mode, target_f, built_in) VALUES (?1, ?2, ?3, 0);",
        params![name, mode.as_str(), target_f],
    )?;

    get_profile(conn, conn.last_insert_rowid() as i64)
}

pub fn update_profile(
    conn: &Connection,
    user: &User,
    id: i64,
    name: &str,
    mode: Mode,
    target_f: i32,
) -> Result<Profile, ProfileError> {
    require_homeowner(user)?;
    get_profile(conn, id)?; // NotFound before anything else changes
    check_name(name)?;
    check_target(target_f)?;

    // A duplicate name is rejected by the UNIQUE constraint on profiles.name
    conn.execute(
        "UPDATE profiles SET name = ?1, mode = ?2, target_f = ?3 WHERE id = ?4",
        params![name, mode.as_str(), target_f, id],
    )?;

    get_profile(conn, id)
}

pub fn delete_profile(conn: &Connection, user: &User, id: i64) -> Result<(), ProfileError> {
    require_homeowner(user)?;
    let profile = get_profile(conn, id)?;
    if profile.built_in {
        return Err(ProfileError::BuiltInProtected);
    }

    conn.execute("DELETE FROM profiles WHERE id = ?1", params![id])?;
    Ok(())
}

// Any role may activate a profile. A guest can ONLY pick from the existing profiles; they never
// choose their own temperature (that is a separate action guests are not allowed to do).
// Returns the profile to apply; thermostat::apply_profile copies it into the live settings.
pub fn activate_profile(conn: &Connection, _user: &User, id: i64) -> Result<Profile, ProfileError> {
    get_profile(conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_profiles(&conn).unwrap();
        conn
    }

    fn user(role: Role) -> User {
        User {
            id: 1,
            username: "tester".to_string(),
            role,
            valid_until: None,
            failed_attempts: 0,
            //is_locked: false,
        }
    }

    fn id_of(conn: &Connection, name: &str) -> i64 {
        list_profiles(conn).unwrap().into_iter().find(|p| p.name == name).unwrap().id
    }

    #[test]
    fn seeds_four_built_ins_once() {
        let conn = setup();
        init_profiles(&conn).unwrap(); // second call must not duplicate
        let profiles = list_profiles(&conn).unwrap();
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Home", "Away", "Sleep", "Vacation"]);
        assert!(profiles.iter().all(|p| p.built_in));
    }

    #[test]
    fn built_in_targets_are_valid() {
        for (_, _, target) in BUILT_INS {
            assert!(check_target(target).is_ok());
        }
    }

    #[test]
    fn homeowner_can_create_profile() {
        let conn = setup();
        let p = create_profile(&conn, &user(Role::Homeowner), "Movie Night", Mode::Cool, 70).unwrap();
        assert_eq!(p.name, "Movie Night");
        assert_eq!(p.mode, Mode::Cool);
        assert!(!p.built_in);
    }

    #[test]
    fn guest_and_technician_cannot_manage_profiles() {
        let conn = setup();
        for role in [Role::Guest, Role::Technician] {
            let u = user(role);
            assert!(matches!(create_profile(&conn, &u, "Nope", Mode::Heat, 70), Err(ProfileError::NotAllowed)));
            assert!(matches!(update_profile(&conn, &u, 1, "Nope", Mode::Heat, 70), Err(ProfileError::NotAllowed)));
            assert!(matches!(delete_profile(&conn, &u, 1), Err(ProfileError::NotAllowed)));
        }
    }

    #[test]
    fn rejects_bad_names() {
        let conn = setup();
        let h = user(Role::Homeowner);
        for bad in ["", " lead", "trail ", "bad\nname", "semi;colon", &"x".repeat(33)] {
            assert!(matches!(create_profile(&conn, &h, bad, Mode::Auto, 70), Err(ProfileError::InvalidName)), "{bad:?}");
        }
    }

    #[test]
    fn rejects_out_of_range_targets() {
        let conn = setup();
        let h = user(Role::Homeowner);
        assert!(matches!(create_profile(&conn, &h, "Cold", Mode::Cool, 20), Err(ProfileError::InvalidTarget)));
        assert!(matches!(create_profile(&conn, &h, "Hot", Mode::Heat, 150), Err(ProfileError::InvalidTarget)));
        assert!(create_profile(&conn, &h, "Min", Mode::Heat, 50).is_ok());
        assert!(create_profile(&conn, &h, "Max", Mode::Heat, 90).is_ok());
    }

    #[test]
    fn rejects_duplicate_names_case_insensitively() {
        let conn = setup();
        let h = user(Role::Homeowner);
        assert!(matches!(create_profile(&conn, &h, "home", Mode::Auto, 70), Err(ProfileError::DatabaseError(_))));
    }

    #[test]
    fn built_in_can_be_edited_but_not_deleted() {
        let conn = setup();
        let h = user(Role::Homeowner);
        let id = id_of(&conn, "Sleep");

        let edited = update_profile(&conn, &h, id, "Sleep", Mode::Heat, 66).unwrap();
        assert_eq!(edited.target_f, 66);
        assert!(edited.built_in);

        assert!(matches!(delete_profile(&conn, &h, id), Err(ProfileError::BuiltInProtected)));
    }

    #[test]
    fn update_can_keep_own_name_but_not_take_anothers() {
        let conn = setup();
        let h = user(Role::Homeowner);
        let id = id_of(&conn, "Home");
        assert!(update_profile(&conn, &h, id, "Home", Mode::Auto, 73).is_ok());
        assert!(matches!(update_profile(&conn, &h, id, "Away", Mode::Auto, 73), Err(ProfileError::DatabaseError(_))));
    }

    #[test]
    fn custom_profile_can_be_deleted() {
        let conn = setup();
        let h = user(Role::Homeowner);
        let p = create_profile(&conn, &h, "Temp", Mode::Fan, 70).unwrap();
        delete_profile(&conn, &h, p.id).unwrap();
        assert!(matches!(activate_profile(&conn, &h, p.id), Err(ProfileError::NotFound)));
    }

    #[test]
    fn every_role_can_activate_a_profile() {
        let conn = setup();
        for name in ["Home", "Away", "Sleep", "Vacation"] {
            let id = id_of(&conn, name);
            for role in [Role::Homeowner, Role::Technician, Role::Guest] {
                assert_eq!(activate_profile(&conn, &user(role), id).unwrap().name, name);
            }
        }
    }

    #[test]
    fn activating_unknown_profile_fails() {
        let conn = setup();
        assert!(matches!(activate_profile(&conn, &user(Role::Guest), 9999), Err(ProfileError::NotFound)));
    }
}
