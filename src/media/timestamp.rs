extern crate chrono;

use std::fmt;

use chrono::{NaiveDateTime, NaiveTime, Timelike};

#[derive(Clone, Copy)]
pub struct Timestamp {
    date: NaiveDateTime,
    time: NaiveTime,
    is_date: bool,
}

impl Timestamp {
    pub fn new() -> Self {
        Timestamp {
            date: NaiveDateTime::from_timestamp(0, 0),
            time: NaiveTime::from_num_seconds_from_midnight(0, 0),
            is_date: false,
        }
    }

    fn from_sec_nano(sec: i64, nano: u32) -> Self {
        if sec > 24i64 * 3600i64 {
            // sec part larger than one day
            Timestamp {
                date: NaiveDateTime::from_timestamp(sec, nano),
                time: NaiveTime::from_num_seconds_from_midnight(0, 0),
                is_date: true
            }
        } else {
            Timestamp {
                date: NaiveDateTime::from_timestamp(0, 0),
                time: NaiveTime::from_num_seconds_from_midnight(sec as u32, nano),
                is_date: false
            }
        }
    }

    pub fn from_sec_time_factor(sec: i64, time_factor: f64) -> Self {
        let sec_f = sec.abs() as f64 * time_factor;
        let sec = sec_f.trunc() as i64;
        Timestamp::from_sec_nano(
            sec as i64,
            (sec_f.fract() * 1_000_000_000f64) as u32
        )
    }

    pub fn from_signed_nano(nano: i64) -> Self {
        let sec_f = nano.abs() as f64 / 1_000_000_000f64;
        let sec = sec_f.trunc() as i64;
        let nano = (nano - (sec * 1_000_000_000i64)).abs() as u32;
        Timestamp::from_sec_nano(sec, nano)
    }

    pub fn from_nano(nano: u64) -> Self {
        let sec_f = nano as f64 / 1_000_000_000f64;
        let sec = sec_f.trunc() as u64;
        let nano = (nano - (sec * 1_000_000_000u64)) as u32;
        Timestamp::from_sec_nano(sec as i64, nano)
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
