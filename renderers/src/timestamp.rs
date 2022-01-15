use std::{
    cmp::Ordering,
    error::Error,
    fmt,
    ops::{Add, Sub},
};

use metadata::{Duration, Timestamp4Humans};

use super::SampleIndex;

#[derive(Clone, Copy, Default, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(u64);

impl Timestamp {
    pub fn new(value: u64) -> Self {
        Timestamp(value)
    }

    pub fn for_humans(self) -> Timestamp4Humans {
        Timestamp4Humans::from_nano(self.0)
    }

    #[track_caller]
    pub fn halfway_to(self, other: Timestamp) -> Timestamp {
        Timestamp((self.0 + other.0) / 2)
    }

    #[track_caller]
    pub fn sample_index(self, sample_duration: Duration) -> SampleIndex {
        SampleIndex::new((self.0 / sample_duration.as_u64()) as usize)
    }

    pub fn as_f64(self) -> f64 {
        self.0 as f64
    }

    pub fn as_i64(self) -> i64 {
        self.0 as i64
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    // FIXME there seems to be a trait for this now
    #[must_use = "this returns the result of the operation, without modifying the original"]
    pub fn saturating_sub(self, rhs: Duration) -> Timestamp {
        Timestamp::new(self.0.saturating_sub(rhs.as_u64()))
    }
}

impl From<u64> for Timestamp {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<i64> for Timestamp {
    fn from(value: i64) -> Self {
        Self(value as u64)
    }
}

impl From<Duration> for Timestamp {
    fn from(duration: Duration) -> Self {
        Self(duration.as_u64())
    }
}

impl From<Timestamp> for gst::ClockTime {
    fn from(ts: Timestamp) -> Self {
        gst::ClockTime::from_nseconds(ts.as_u64())
    }
}

impl From<gst::ClockTime> for Timestamp {
    fn from(clock_time: gst::ClockTime) -> Self {
        Timestamp(clock_time.nseconds())
    }
}

#[derive(Debug)]
pub struct TimestampTryFromError;

impl fmt::Display for TimestampTryFromError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Timestamp from other type failed")
    }
}

impl Error for TimestampTryFromError {}

impl TryFrom<Option<gst::ClockTime>> for Timestamp {
    type Error = TimestampTryFromError;
    fn try_from(opt_clock_time: Option<gst::ClockTime>) -> Result<Self, Self::Error> {
        opt_clock_time
            .map(Timestamp::from)
            .ok_or(TimestampTryFromError)
    }
}

impl Sub for Timestamp {
    type Output = Duration;

    #[track_caller]
    fn sub(self, rhs: Timestamp) -> Duration {
        Duration::from_nanos(self.0 - rhs.0)
    }
}

impl Add<Duration> for Timestamp {
    type Output = Timestamp;

    #[track_caller]
    fn add(self, rhs: Duration) -> Timestamp {
        Timestamp(self.0 + rhs.as_u64())
    }
}

impl Sub<Duration> for Timestamp {
    type Output = Timestamp;

    #[track_caller]
    fn sub(self, rhs: Duration) -> Timestamp {
        Timestamp(self.0 - rhs.as_u64())
    }
}

impl PartialOrd<Duration> for Timestamp {
    fn partial_cmp(&self, rhs: &Duration) -> Option<Ordering> {
        Some(self.0.cmp(&rhs.as_u64()))
    }
}

impl PartialEq<Duration> for Timestamp {
    fn eq(&self, rhs: &Duration) -> bool {
        self.0 == rhs.as_u64()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ts {}", self.0)
    }
}
