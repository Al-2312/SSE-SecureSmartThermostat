// The HVAC controller: given the mode, target and indoor temperature, decide what the equipment
// does. A pure function (no database, no clock) so every case is easy to test.

use crate::profiles::Mode;

// How far the indoor temperature must drift past the target before heating/cooling switches on.
// Without it the equipment would flick on and off every few seconds around the target.
pub const DEADBAND_F: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Output {
    Idle,
    Heating,
    Cooling,
    FanOnly,
}

impl Output {
    pub fn as_str(&self) -> &'static str {
        match self {
            Output::Idle => "Idle",
            Output::Heating => "Heating",
            Output::Cooling => "Cooling",
            Output::FanOnly => "FanOnly",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Idle" => Some(Output::Idle),
            "Heating" => Some(Output::Heating),
            "Cooling" => Some(Output::Cooling),
            "FanOnly" => Some(Output::FanOnly),
            _ => None,
        }
    }
}

// Heating starts once indoor is DEADBAND_F below target and keeps running until it reaches the
// target (cooling mirrors this). If the sensors are faulty, heating and cooling stop: running
// blind is less safe than doing nothing. Fan-only doesn't depend on readings, so it keeps going.
pub fn decide(mode: Mode, target_f: i32, indoor_f: f64, current: Output, sensor_ok: bool) -> Output {
    let target = f64::from(target_f);

    let heat = sensor_ok
        && ((current == Output::Heating && indoor_f < target) || indoor_f <= target - DEADBAND_F);
    let cool = sensor_ok
        && ((current == Output::Cooling && indoor_f > target) || indoor_f >= target + DEADBAND_F);

    match mode {
        Mode::Off => Output::Idle,
        Mode::Fan => Output::FanOnly,
        Mode::Heat => if heat { Output::Heating } else { Output::Idle },
        Mode::Cool => if cool { Output::Cooling } else { Output::Idle },
        Mode::Auto => {
            if heat {
                Output::Heating
            } else if cool {
                Output::Cooling
            } else {
                Output::Idle
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Output::*;

    #[test]
    fn off_is_always_idle_and_fan_is_always_fan() {
        for indoor in [40.0, 72.0, 100.0] {
            assert_eq!(decide(Mode::Off, 72, indoor, Heating, true), Idle);
            assert_eq!(decide(Mode::Fan, 72, indoor, Idle, true), FanOnly);
            assert_eq!(decide(Mode::Fan, 72, indoor, Idle, false), FanOnly);
        }
    }

    #[test]
    fn heat_starts_only_past_the_deadband() {
        assert_eq!(decide(Mode::Heat, 72, 71.5, Idle, true), Idle);
        assert_eq!(decide(Mode::Heat, 72, 71.0, Idle, true), Heating);
        assert_eq!(decide(Mode::Heat, 72, 80.0, Idle, true), Idle); // never cools
    }

    #[test]
    fn heat_keeps_running_until_target_reached() {
        assert_eq!(decide(Mode::Heat, 72, 71.9, Heating, true), Heating);
        assert_eq!(decide(Mode::Heat, 72, 72.0, Heating, true), Idle);
    }

    #[test]
    fn cool_mirrors_heat() {
        assert_eq!(decide(Mode::Cool, 72, 72.5, Idle, true), Idle);
        assert_eq!(decide(Mode::Cool, 72, 73.0, Idle, true), Cooling);
        assert_eq!(decide(Mode::Cool, 72, 72.1, Cooling, true), Cooling);
        assert_eq!(decide(Mode::Cool, 72, 72.0, Cooling, true), Idle);
        assert_eq!(decide(Mode::Cool, 72, 60.0, Idle, true), Idle); // never heats
    }

    #[test]
    fn auto_heats_or_cools_as_needed() {
        assert_eq!(decide(Mode::Auto, 72, 65.0, Idle, true), Heating);
        assert_eq!(decide(Mode::Auto, 72, 80.0, Idle, true), Cooling);
        assert_eq!(decide(Mode::Auto, 72, 72.4, Idle, true), Idle);
    }

    #[test]
    fn sensor_fault_stops_heating_and_cooling() {
        assert_eq!(decide(Mode::Heat, 72, 60.0, Heating, false), Idle);
        assert_eq!(decide(Mode::Cool, 72, 90.0, Cooling, false), Idle);
        assert_eq!(decide(Mode::Auto, 72, 60.0, Idle, false), Idle);
    }

    #[test]
    fn output_text_round_trips() {
        for o in [Idle, Heating, Cooling, FanOnly] {
            assert_eq!(Output::from_str(o.as_str()), Some(o));
        }
        assert_eq!(Output::from_str("Melting"), None);
    }
}
