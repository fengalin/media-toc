extern crate chrono;

use std::clone::Clone;

use std::fmt;

use chrono::{NaiveTime, Timelike};

#[derive(Copy)]
pub struct Timestamp {
    timestamp: NaiveTime,
    is_positive: bool,
}

impl Timestamp {
    pub fn new() -> Self {
        Timestamp {
            timestamp: NaiveTime::from_num_seconds_from_midnight(0, 0),
            is_positive: true,
        }
    }

    pub fn from_sec_time_factor(sec: i64, time_factor: f64) -> Self {
        let sec_f = sec.abs() as f64 * time_factor;
        Timestamp {
            timestamp: NaiveTime::from_num_seconds_from_midnight(
                sec_f.trunc() as u32,
                (sec_f.fract() * 1_000_000_000f64) as u32
            ),
            is_positive: if sec >= 0 { true } else { false },
        }
    }

    pub fn from_nano(nano: i64) -> Self {
        let sec_f = nano.abs() as f64 / 1_000_000_000f64;
        Timestamp {
            timestamp: NaiveTime::from_num_seconds_from_midnight(
                sec_f.trunc() as u32,
                (sec_f.fract() * 1_000_000_000f64) as u32
            ),
            is_positive: if nano >= 0 { true } else { false },
        }
    }
}

impl Clone for Timestamp {
    fn clone(&self) -> Self { *self }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut format = String::new();
        if self.timestamp.hour() > 0 {
            format = "%H:".to_owned();
        }
        format += "%M:%S%.3f";

        write!(f, "{}{}",
            if self.is_positive { "".to_owned() } else  { "-".to_owned() },
            self.timestamp.format(&format).to_string()
        )
    }
}
