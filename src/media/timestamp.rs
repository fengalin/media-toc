extern crate chrono;

use std::fmt;

use chrono::NaiveTime;

pub struct Timestamp {
    timestamp: NaiveTime,
    is_positive: bool,
}

impl Timestamp {
    pub fn new() -> Timestamp {
        Timestamp {
            timestamp: NaiveTime::from_num_seconds_from_midnight(0, 0),
            is_positive: true,
        }
    }

    pub fn from_sec_time_factor(sec: i64, time_factor: f64) -> Timestamp {
        println!("from_sec_time_factor: {}, {}", sec, time_factor);
        let sec_f = sec.abs() as f64 * time_factor;
        Timestamp {
            timestamp: NaiveTime::from_num_seconds_from_midnight(
                sec_f.trunc() as u32,
                (sec_f.fract() * 1_000_000f64) as u32
            ),
            is_positive: if sec >= 0 { true } else { false },
        }
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}",
            if self.is_positive { "".to_owned() } else  { "-".to_owned() },
            self.timestamp.format("%H:%M:%S%.3f").to_string()
        )
    }
}
