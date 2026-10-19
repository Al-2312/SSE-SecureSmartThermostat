// Reads the simulated-environment override file (sim/environment.txt by default).
// Format: one `key = value` per line. Blank lines and anything after '#' are ignored.
// Every key is optional: a key that is present overrides the generated weather (see sim.rs).
// The file is untrusted input (anyone can edit it while the program runs), so every value is
// range-checked and anything unexpected is rejected instead of guessed at.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

const MAX_FILE_BYTES: u64 = 4096;

// Written on first run so there's always a file to edit. Everything is commented out, so it
// overrides nothing until a line is uncommented.
pub const ENV_TEMPLATE: &str = "\
# Simulated outside world. Edit this while the thermostat is running: the next screen picks it up.
#
# Weather is generated automatically. Uncomment a line (remove the leading #) to override that
# value; comment it out again to go back to the generated value.
# A bad or out-of-range value counts as a sensor fault: heating and cooling stop until it's fixed.
#
# outdoor_f = 41             # -60 to 130
# outdoor_humidity_pct = 80  # 0 to 100
# precipitation_mm_hr = 2.5  # 0 to 100
# co_ppm = 3                 # 0 to 1000
# indoor_humidity_pct = 45   # 0 to 100
";

// Creates the template (and its folder) if the file doesn't exist. Returns true if it created one.
// Never overwrites: create_new fails if the file is already there, even if it appeared just now.
pub fn create_template_if_missing(path: &Path) -> std::io::Result<bool> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir)?;
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(ENV_TEMPLATE.as_bytes())?;
            Ok(true)
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EnvOverrides {
    pub outdoor_f: Option<f64>,
    pub outdoor_humidity_pct: Option<f64>,
    pub precipitation_mm_hr: Option<f64>,
    pub co_ppm: Option<f64>,
    pub indoor_humidity_pct: Option<f64>,
}

#[derive(Debug)]
pub enum SensorError {
    Io(std::io::Error),
    TooLarge,
    BadLine { line: usize },
    UnknownKey { line: usize, key: String },
    DuplicateKey { line: usize, key: String },
    InvalidValue { line: usize, key: String, raw: String },
    OutOfRange { line: usize, key: String, value: f64, min: f64, max: f64 },
}

impl fmt::Display for SensorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SensorError::Io(e) => write!(f, "couldn't read the environment file: {e}"),
            SensorError::TooLarge => write!(f, "environment file is larger than {MAX_FILE_BYTES} bytes"),
            SensorError::BadLine { line } => write!(f, "line {line}: expected `key = value`"),
            SensorError::UnknownKey { line, key } => write!(f, "line {line}: unknown key \"{key}\""),
            SensorError::DuplicateKey { line, key } => write!(f, "line {line}: \"{key}\" is set twice"),
            SensorError::InvalidValue { line, key, raw } => {
                write!(f, "line {line}: {key} = \"{raw}\" is not a finite number")
            }
            SensorError::OutOfRange { line, key, value, min, max } => {
                write!(f, "line {line}: {key} = {value} is outside {min} to {max}")
            }
        }
    }
}

// A missing file is not an error: it just means "no overrides, use the generated weather".
pub fn read_environment(path: &Path) -> Result<Option<EnvOverrides>, SensorError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(SensorError::Io(e)),
    };

    // Read at most one byte past the limit so a huge file can't eat memory
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(SensorError::Io)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(SensorError::TooLarge);
    }

    parse_environment(&text).map(Some)
}

pub fn parse_environment(text: &str) -> Result<EnvOverrides, SensorError> {
    let mut overrides = EnvOverrides::default();

    for (i, raw_line) in text.lines().enumerate() {
        let line = i + 1;
        let content = raw_line.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }

        let Some((key, value)) = content.split_once('=') else {
            return Err(SensorError::BadLine { line });
        };
        let (key, value) = (key.trim(), value.trim());

        let Some((slot, min, max)) = field(&mut overrides, key) else {
            return Err(SensorError::UnknownKey { line, key: clean(key) });
        };
        if slot.is_some() {
            return Err(SensorError::DuplicateKey { line, key: clean(key) });
        }

        // f64 parsing accepts "NaN" and "inf", so finiteness is checked separately
        let parsed = match value.parse::<f64>() {
            Ok(v) if v.is_finite() => v,
            _ => return Err(SensorError::InvalidValue { line, key: clean(key), raw: clean(value) }),
        };
        if !(min..=max).contains(&parsed) {
            return Err(SensorError::OutOfRange { line, key: clean(key), value: parsed, min, max });
        }

        *slot = Some(parsed);
    }

    Ok(overrides)
}

// The allowed keys and their valid ranges
fn field<'a>(o: &'a mut EnvOverrides, key: &str) -> Option<(&'a mut Option<f64>, f64, f64)> {
    match key {
        "outdoor_f" => Some((&mut o.outdoor_f, -60.0, 130.0)),
        "outdoor_humidity_pct" => Some((&mut o.outdoor_humidity_pct, 0.0, 100.0)),
        "precipitation_mm_hr" => Some((&mut o.precipitation_mm_hr, 0.0, 100.0)),
        "co_ppm" => Some((&mut o.co_ppm, 0.0, 1000.0)),
        "indoor_humidity_pct" => Some((&mut o.indoor_humidity_pct, 0.0, 100.0)),
        _ => None,
    }
}

// Text copied from the file into an error message: no control characters (which could inject
// terminal escape codes) and no more than 32 characters
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_keys_with_comments_and_blank_lines() {
        let text = "# weather\n\noutdoor_f = 41.5\noutdoor_humidity_pct=80 # inline comment\n\
                    precipitation_mm_hr = 2.5\nco_ppm = 3\nindoor_humidity_pct = 45\n";
        let o = parse_environment(text).unwrap();
        assert_eq!(o.outdoor_f, Some(41.5));
        assert_eq!(o.outdoor_humidity_pct, Some(80.0));
        assert_eq!(o.precipitation_mm_hr, Some(2.5));
        assert_eq!(o.co_ppm, Some(3.0));
        assert_eq!(o.indoor_humidity_pct, Some(45.0));
    }

    #[test]
    fn missing_keys_stay_none() {
        let o = parse_environment("co_ppm = 5").unwrap();
        assert_eq!(o.outdoor_f, None);
        assert_eq!(o.co_ppm, Some(5.0));
        assert_eq!(parse_environment("").unwrap(), EnvOverrides::default());
    }

    #[test]
    fn rejects_line_without_equals() {
        assert!(matches!(parse_environment("outdoor_f 41"), Err(SensorError::BadLine { line: 1 })));
    }

    #[test]
    fn rejects_unknown_and_duplicate_keys() {
        assert!(matches!(parse_environment("indoor_f = 70"), Err(SensorError::UnknownKey { .. })));
        assert!(matches!(
            parse_environment("co_ppm = 1\nco_ppm = 2"),
            Err(SensorError::DuplicateKey { line: 2, .. })
        ));
    }

    #[test]
    fn rejects_non_numbers_nan_and_infinity() {
        for bad in ["outdoor_f = warm", "outdoor_f = NaN", "outdoor_f = inf", "outdoor_f ="] {
            assert!(matches!(parse_environment(bad), Err(SensorError::InvalidValue { .. })), "{bad}");
        }
    }

    #[test]
    fn rejects_out_of_range_values() {
        assert!(matches!(parse_environment("outdoor_f = 200"), Err(SensorError::OutOfRange { .. })));
        assert!(matches!(parse_environment("co_ppm = -1"), Err(SensorError::OutOfRange { .. })));
        assert!(parse_environment("outdoor_humidity_pct = 100").is_ok());
    }

    #[test]
    fn error_text_strips_control_characters() {
        let err = parse_environment("outdoor_f = \x1b[31mred").unwrap_err();
        assert!(!err.to_string().contains('\x1b'));
    }

    #[test]
    fn missing_file_means_no_overrides() {
        let path = std::env::temp_dir().join("sse_thermostat_definitely_missing.txt");
        assert!(matches!(read_environment(&path), Ok(None)));
    }

    #[test]
    fn template_overrides_nothing() {
        assert_eq!(parse_environment(ENV_TEMPLATE).unwrap(), EnvOverrides::default());
    }

    #[test]
    fn template_is_created_once_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("sse_thermostat_tpl_{}", std::process::id()));
        let path = dir.join("sim").join("environment.txt");
        std::fs::remove_dir_all(&dir).ok();

        assert!(create_template_if_missing(&path).unwrap()); // creates folder + file
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ENV_TEMPLATE);

        std::fs::write(&path, "outdoor_f = 10").unwrap();
        assert!(!create_template_if_missing(&path).unwrap()); // already there: left alone
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "outdoor_f = 10");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_oversized_file() {
        let path = std::env::temp_dir().join(format!("sse_thermostat_big_{}.txt", std::process::id()));
        std::fs::write(&path, "#".repeat(5000)).unwrap();
        let result = read_environment(&path);
        std::fs::remove_file(&path).ok();
        assert!(matches!(result, Err(SensorError::TooLarge)));
    }
}
