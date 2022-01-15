use std::{
    fmt,
    ops::{Div, DivAssign, Mul, MulAssign},
};

// FIXME: consider moving to std::time::Duration when `div_duration` is stabilized.

#[derive(Clone, Copy, Default, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Duration(u64);

impl Duration {
    pub const fn from_nanos(nanos: u64) -> Self {
        Duration(nanos)
    }

    #[track_caller]
    pub const fn from_secs(secs: u64) -> Self {
        Duration(secs * 1_000_000_000u64)
    }

    #[track_caller]
    pub const fn from_millis(millis: u64) -> Self {
        Duration(millis * 1_000_000u64)
    }

    #[track_caller]
    pub const fn from_frequency(freq: u64) -> Self {
        Duration(1_000_000_000u64 / freq)
    }

    pub fn as_f64(self) -> f64 {
        self.0 as f64
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn as_i64(self) -> i64 {
        self.0 as i64
    }

    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<Duration> for u64 {
    fn from(duration: Duration) -> Self {
        duration.0
    }
}

impl From<Duration> for gst::ClockTime {
    fn from(duration: Duration) -> Self {
        gst::ClockTime::from_nseconds(duration.0)
    }
}

impl From<gst::ClockTime> for Duration {
    fn from(clock_time: gst::ClockTime) -> Self {
        Duration(clock_time.nseconds())
    }
}

impl Div for Duration {
    type Output = Duration;

    #[track_caller]
    fn div(self, rhs: Duration) -> Self::Output {
        Duration(self.0 / rhs.0)
    }
}

impl Div<u64> for Duration {
    type Output = Duration;

    #[track_caller]
    fn div(self, rhs: u64) -> Self::Output {
        Duration(self.0 / rhs)
    }
}

impl DivAssign<u64> for Duration {
    #[track_caller]
    fn div_assign(&mut self, rhs: u64) {
        *self = Duration(self.0 / rhs);
    }
}

impl Mul<u64> for Duration {
    type Output = Duration;

    #[track_caller]
    fn mul(self, rhs: u64) -> Self::Output {
        Duration(self.0 * rhs)
    }
}

impl MulAssign<u64> for Duration {
    #[track_caller]
    fn mul_assign(&mut self, rhs: u64) {
        *self = Duration(self.0 * rhs);
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "idx range {}", self.0)
    }
}
