use std::fmt;
use std::ops::{Add, AddAssign, Sub};

use super::Timestamp;

#[derive(Clone, Copy, Default, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SampleIndex(usize);

impl SampleIndex {
    pub fn new(inner: usize) -> Self {
        SampleIndex(inner)
    }

    pub fn from_ts(ts: Timestamp, sample_duration: u64) -> Self {
        SampleIndex((ts.as_u64() / sample_duration) as usize)
    }

    pub fn get_aligned(&self, step: SampleIndex) -> SampleIndex {
        SampleIndex(self.0 / step.0 * step.0)
    }

    pub fn get_step_index(&self, sample_step: SampleIndex) -> usize {
        self.0 / sample_step.0
    }

    pub fn get_ts(&self, sample_duration: u64) -> Timestamp {
        Timestamp::new(self.0 as u64 * sample_duration)
    }

    pub fn as_f64(&self) -> f64 {
        self.0 as f64
    }

    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    pub fn as_u64(&self) -> u64 {
        self.0 as u64
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }

    pub fn dec(&mut self) {
        *self = SampleIndex(self.0 - 1);
    }

    pub fn inc(&mut self) {
        *self = SampleIndex(self.0 + 1);
    }
}

impl From<usize> for SampleIndex {
    fn from(inner: usize) -> Self {
        Self(inner)
    }
}

impl From<u64> for SampleIndex {
    fn from(inner: u64) -> Self {
        Self(inner as usize)
    }
}

impl Add for SampleIndex {
    type Output = SampleIndex;

    fn add(self, rhs: SampleIndex) -> SampleIndex {
        SampleIndex(self.0 + rhs.0)
    }
}

impl AddAssign for SampleIndex {
    fn add_assign(&mut self, rhs: SampleIndex) {
        *self = SampleIndex(self.0 + rhs.0);
    }
}

impl Sub for SampleIndex {
    type Output = SampleIndex;

    fn sub(self, rhs: SampleIndex) -> SampleIndex {
        SampleIndex(self.0 - rhs.0)
    }
}

impl fmt::Display for SampleIndex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "idx {}", self.0)
    }
}

#[cfg(test)]
impl From<i32> for SampleIndex {
    fn from(inner: i32) -> Self {
        Self(inner as usize)
    }
}
