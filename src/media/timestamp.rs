use std::fmt;
use std::ops::{Add, AddAssign, Sub};

use crate::metadata::Timestamp4Humans;

use super::SampleIndex;

#[derive(Clone, Copy, Default, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(u64);

impl Timestamp {
    pub fn new(inner: u64) -> Self {
        Timestamp(inner)
    }

    pub fn get_4_humans(&self) -> Timestamp4Humans {
        Timestamp4Humans::from_nano(self.0)
    }

    pub fn get_sample_index(&self, sample_duration: u64) -> SampleIndex {
        SampleIndex::new((self.0 / sample_duration) as usize)
    }

    pub fn as_f64(&self) -> f64 {
        self.0 as f64
    }

    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl From<u64> for Timestamp {
    fn from(inner: u64) -> Self {
        Self(inner)
    }
}

impl From<i64> for Timestamp {
    fn from(inner: i64) -> Self {
        Self(inner as u64)
    }
}

impl Add for Timestamp {
    type Output = Timestamp;

    fn add(self, rhs: Timestamp) -> Timestamp {
        Timestamp(self.0 + rhs.0)
    }
}

impl AddAssign for Timestamp {
    fn add_assign(&mut self, rhs: Timestamp) {
        *self = Timestamp(self.0 + rhs.0);
    }
}

impl Sub for Timestamp {
    type Output = Timestamp;

    fn sub(self, rhs: Timestamp) -> Timestamp {
        Timestamp(self.0 - rhs.0)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ts {}", self.0)
    }
}
