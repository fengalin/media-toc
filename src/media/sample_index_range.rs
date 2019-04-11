use std::fmt;

#[derive(Clone, Copy, Default, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SampleIndexRange(usize);

impl SampleIndexRange {
    pub const fn new(value: usize) -> Self {
        SampleIndexRange(value)
    }

    // FIXME: update when Duration is available
    pub fn from_duration(duration: u64, sample_duration: u64) -> Self {
        SampleIndexRange((duration / sample_duration) as usize)
    }

    // FIXME: update when Duration is available
    pub fn get_duration(&self, sample_duration: u64) -> u64 {
        self.0 as u64 * sample_duration
    }

    pub fn get_scaled<T: Into<usize>>(&self, num: T, denom: T) -> Self {
        SampleIndexRange(num.into() / denom.into() * self.0)
    }

    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    pub fn as_usize(&self) -> usize {
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
