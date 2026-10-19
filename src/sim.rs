// The simulated house and outside world.
// Weather is generated from the simulated clock (a daily temperature curve plus slower drifts), so
// it changes on its own; any value set in sim/environment.txt overrides the generated one.
// The indoor temperature is modelled: it leaks toward the outdoor temperature and the HVAC pushes it.

use std::f64::consts::TAU;

use crate::hvac::Output;
use crate::sensors::EnvOverrides;

// Each simulated minute the house loses this fraction of the indoor/outdoor gap
pub const LEAK_PER_MIN: f64 = 0.005;
// Degrees per simulated minute the equipment adds or removes
pub const HEAT_F_PER_MIN: f64 = 0.5;
pub const COOL_F_PER_MIN: f64 = 0.5;

const DAY_SECS: f64 = 86_400.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Environment {
    pub outdoor_f: f64,
    pub outdoor_humidity_pct: f64,
    pub precipitation_mm_hr: f64,
    pub co_ppm: f64,
    pub indoor_humidity_pct: f64,
}

// A sine wave with the given period, between -1 and 1
fn wave(t_secs: f64, period_secs: f64, phase: f64) -> f64 {
    (TAU * (t_secs / period_secs + phase)).sin()
}

// Deterministic "weather" for a moment in simulated time: the same moment always gives the same
// weather, which keeps it smooth between ticks and makes it testable.
pub fn generate(sim_ms: i64) -> Environment {
    let t = sim_ms as f64 / 1000.0;

    let daily = wave(t, DAY_SECS, -0.375); // warmest around 15:00 UTC, coldest before dawn
    let front = wave(t, DAY_SECS * 3.7, 0.0); // weather fronts lasting a few days
    let wobble = wave(t, 3600.0 * 5.3, 0.2); // small changes over a few hours

    let outdoor_f = 55.0 + 12.0 * daily + 8.0 * front + 1.5 * wobble;
    let outdoor_humidity_pct = (60.0 - 15.0 * daily + 20.0 * front).clamp(15.0, 100.0);
    let precipitation_mm_hr = ((outdoor_humidity_pct - 85.0) * 0.4).max(0.0);
    let co_ppm = 2.0 + wave(t, 3600.0 * 7.1, 0.5); // normal household background: 1 to 3 ppm
    let indoor_humidity_pct = 45.0 + 5.0 * front;

    Environment { outdoor_f, outdoor_humidity_pct, precipitation_mm_hr, co_ppm, indoor_humidity_pct }
}

impl Environment {
    pub fn with_overrides(self, o: &EnvOverrides) -> Self {
        Environment {
            outdoor_f: o.outdoor_f.unwrap_or(self.outdoor_f),
            outdoor_humidity_pct: o.outdoor_humidity_pct.unwrap_or(self.outdoor_humidity_pct),
            precipitation_mm_hr: o.precipitation_mm_hr.unwrap_or(self.precipitation_mm_hr),
            co_ppm: o.co_ppm.unwrap_or(self.co_ppm),
            indoor_humidity_pct: o.indoor_humidity_pct.unwrap_or(self.indoor_humidity_pct),
        }
    }

    pub fn precipitation_label(&self) -> &'static str {
        if self.precipitation_mm_hr <= 0.0 {
            "None"
        } else if self.outdoor_f <= 32.0 {
            "Snow"
        } else if self.precipitation_mm_hr < 2.5 {
            "Light rain"
        } else {
            "Rain"
        }
    }
}

// Moves the indoor temperature forward by dt_secs of simulated time
pub fn step_indoor(indoor_f: f64, outdoor_f: f64, output: Output, dt_secs: f64) -> f64 {
    let hvac = match output {
        Output::Heating => HEAT_F_PER_MIN,
        Output::Cooling => -COOL_F_PER_MIN,
        Output::FanOnly | Output::Idle => 0.0,
    };
    indoor_f + ((outdoor_f - indoor_f) * LEAK_PER_MIN + hvac) * dt_secs / 60.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heating_raises_and_cooling_lowers_temperature() {
        assert!(step_indoor(70.0, 40.0, Output::Heating, 60.0) > 70.0);
        assert!(step_indoor(70.0, 95.0, Output::Cooling, 60.0) < 70.0);
    }

    #[test]
    fn idle_house_drifts_toward_outdoor_without_overshooting() {
        let mut indoor = 70.0;
        for _ in 0..(24 * 60) {
            indoor = step_indoor(indoor, 40.0, Output::Idle, 60.0);
        }
        assert!(indoor < 45.0 && indoor > 40.0, "{indoor}");
        assert_eq!(step_indoor(70.0, 70.0, Output::FanOnly, 600.0), 70.0);
    }

    #[test]
    fn generated_weather_stays_in_sensible_ranges() {
        // Sample every 15 simulated minutes over 10 days
        for i in 0..(10 * 96) {
            let e = generate(i * 15 * 60 * 1000);
            assert!((20.0..=90.0).contains(&e.outdoor_f), "{e:?}");
            assert!((15.0..=100.0).contains(&e.outdoor_humidity_pct), "{e:?}");
            assert!(e.precipitation_mm_hr >= 0.0);
            assert!((1.0..=3.0).contains(&e.co_ppm), "{e:?}");
            assert!((40.0..=50.0).contains(&e.indoor_humidity_pct), "{e:?}");
        }
    }

    #[test]
    fn afternoon_is_warmer_than_early_morning() {
        let day = 10 * 86_400_000; // a day where the slow drifts barely matter within 12 hours
        let morning = generate(day + 3 * 3_600_000).outdoor_f;
        let afternoon = generate(day + 15 * 3_600_000).outdoor_f;
        assert!(afternoon > morning);
    }

    #[test]
    fn overrides_replace_only_the_values_given() {
        let base = generate(0);
        let o = EnvOverrides { outdoor_f: Some(10.0), co_ppm: Some(50.0), ..Default::default() };
        let e = base.with_overrides(&o);
        assert_eq!(e.outdoor_f, 10.0);
        assert_eq!(e.co_ppm, 50.0);
        assert_eq!(e.outdoor_humidity_pct, base.outdoor_humidity_pct);
    }

    #[test]
    fn precipitation_labels() {
        let mut e = generate(0);
        e.precipitation_mm_hr = 0.0;
        assert_eq!(e.precipitation_label(), "None");
        e.precipitation_mm_hr = 1.0;
        e.outdoor_f = 50.0;
        assert_eq!(e.precipitation_label(), "Light rain");
        e.outdoor_f = 30.0;
        assert_eq!(e.precipitation_label(), "Snow");
    }
}
