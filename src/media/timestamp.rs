extern crate chrono;

use std::fmt;

use chrono::{NaiveDateTime, NaiveTime, Timelike};

#[derive(Clone, Copy)]
pub struct Timestamp {
    pub nano: f64,
    date: NaiveDateTime,
    time: NaiveTime,
    is_date: bool,
}

impl Timestamp {
    pub fn new() -> Self {
        Timestamp {
            nano: 0f64,
            date: NaiveDateTime::from_timestamp(0, 0),
            time: NaiveTime::from_num_seconds_from_midnight(0, 0),
            is_date: false,
        }
    }

    pub fn from_nano_f(nano_f: f64) -> Self {
        let sec = (nano_f / 1_000_000_000f64).trunc();
        let nano = nano_f - sec * 1_000_000_000f64;
        if sec > 24f64 * 3600f64 {
            // sec part larger than one day
            Timestamp {
                nano: nano_f,
                date: NaiveDateTime::from_timestamp(sec as i64, nano as u32),
                time: NaiveTime::from_num_seconds_from_midnight(0, 0),
                is_date: true
            }
        } else {
            Timestamp {
                nano: nano_f,
                date: NaiveDateTime::from_timestamp(0, 0),
                time: NaiveTime::from_num_seconds_from_midnight(sec as u32, nano as u32),
                is_date: false
            }
        }
    }

    pub fn from_signed_nano(nano: i64) -> Self {
        Timestamp::from_nano_f(nano as f64)
    }

    pub fn from_nano(nano: u64) -> Self {
        Timestamp::from_nano_f(nano as f64)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.is_date {
            write!(f, "{}",
                self.date.format("%Y/%m/%d %H:%M:%S%.3f").to_string()
            )
        }
        else {
            let mut format = if self.time.hour() > 0 {
                "%H:".to_owned()
            }
            else {
                String::new()
            };

            format += "%M:%S%.3f";

            write!(f, "{}",
                self.time.format(&format).to_string()
            )
        }
    }
}
