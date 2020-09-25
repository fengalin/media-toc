use std::fmt;

use metadata::Duration;

#[derive(Clone, Copy, Default, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SampleIndexRange(usize);

impl SampleIndexRange {
    pub const fn new(value: usize) -> Self {
        SampleIndexRange(value)
    }

    #[track_caller]
    pub fn from_duration(duration: Duration, sample_duration: Duration) -> Self {
        SampleIndexRange((duration / sample_duration).as_usize())
    }

    #[track_caller]
    pub fn duration(self, sample_duration: Duration) -> Duration {
        sample_duration * (self.0 as u64)
    }

    #[track_caller]
    pub fn scale<T: Into<usize>>(self, num: T, denom: T) -> Self {
        SampleIndexRange(num.into() / denom.into() * self.0)
    }

    #[track_caller]
    pub fn step_range(self, sample_step: SampleIndexRange) -> usize {
        self.0 / sample_step.0
    }

    pub fn as_f64(self) -> f64 {
        self.0 as f64
    }

    pub fn as_usize(self) -> usize {
        self.0
    }
}

impl From<usize> for SampleIndexRange {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl fmt::Display for SampleIndexRange {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "idx range {}", self.0)
    }
}
