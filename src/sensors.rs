use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct SensorReading {
    pub values: Vec<f64>,
}

#[derive(Debug)]
pub enum SensorParseError {
    Io(std::io::Error),
    InvalidValue { index: usize, raw: String },
    NonFinite { index: usize, raw: String },
}

impl From<std::io::Error> for SensorParseError {
    fn from(err: std::io::Error) -> Self {
        SensorParseError::Io(err)
    }
}

pub fn read_sensor_file(path: &Path) -> Result<SensorReading, SensorParseError> {
    let contents = fs::read_to_string(path)?;
    parse_sensor_line(contents.trim())
}

pub fn parse_sensor_line(line: &str) -> Result<SensorReading, SensorParseError> {
    let mut values = Vec::new();

    for (index, raw) in line.split(',').enumerate() {
        let raw = raw.trim();

        let value: f64 = raw
            .parse()
            .map_err(|_| SensorParseError::InvalidValue { index, raw: raw.to_string() })?;

        if !value.is_finite() {
            return Err(SensorParseError::NonFinite { index, raw: raw.to_string() });
        }

        values.push(value);
    }

    Ok(SensorReading { values })
}

impl SensorReading {
    // Placeholder accessor until the real sensor order/count is decided —
    // swap for named fields (e.g. `.temperature_f()`) once that's locked in.
    pub fn get(&self, index: usize) -> Option<f64> {
        self.values.get(index).copied()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_line() {
        let reading = parse_sensor_line("98.5,60,123,50").unwrap();
        assert_eq!(reading.values, vec![98.5, 60.0, 123.0, 50.0]);
    }

    #[test]
    fn rejects_non_numeric_value() {
        let err = parse_sensor_line("98.5,oops,123").unwrap_err();
        assert!(matches!(err, SensorParseError::InvalidValue { index: 1, .. }));
    }

    #[test]
    fn rejects_nan_and_infinity() {
        assert!(matches!(
            parse_sensor_line("98.5,NaN,123").unwrap_err(),
            SensorParseError::NonFinite { index: 1, .. }
        ));
        assert!(matches!(
            parse_sensor_line("inf,50").unwrap_err(),
            SensorParseError::NonFinite { index: 0, .. }
        ));
    }
}
