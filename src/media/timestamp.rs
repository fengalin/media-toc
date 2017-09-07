use chrono::{NaiveDateTime, NaiveTime, Timelike};

use std::fmt;

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct Timestamp {
    pub nano: i64,
    date: NaiveDateTime,
    time: NaiveTime,
    is_date: bool,
    is_positive: bool,
}

impl Timestamp {
    pub fn from_nano(nano: i64) -> Self {
        let sec = nano / 1_000_000_000;
        let nano_rem = nano - sec * 1_000_000_000;
        if sec.abs() > 24 * 3_600 * 1_000_000_000 {
            // sec part larger than one day
            Timestamp {
                nano: nano,
                date: NaiveDateTime::from_timestamp(sec as i64, nano_rem.abs() as u32),
                time: NaiveTime::from_num_seconds_from_midnight(0, 0),
                is_date: true,
                is_positive: nano.is_positive()
            }
        } else {
            Timestamp {
                nano: nano,
                date: NaiveDateTime::from_timestamp(0, 0),
                time: NaiveTime::from_num_seconds_from_midnight(sec.abs() as u32, nano_rem.abs() as u32),
                is_date: false,
                is_positive: nano.is_positive()
            }
        }
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
