use std::{
    cmp::Ordering,
    fmt,
    ops::{Add, AddAssign, Sub},
};

use metadata::Duration;

use super::{SampleIndexRange, Timestamp};

#[derive(Clone, Copy, Default, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SampleIndex(usize);

impl SampleIndex {
    pub fn new(value: usize) -> Self {
        SampleIndex(value)
    }

    #[track_caller]
    pub fn from_ts(ts: Timestamp, sample_duration: Duration) -> Self {
        SampleIndex((ts.as_u64() / sample_duration.as_u64()) as usize)
    }

    #[track_caller]
    pub fn snap_to(self, sample_step: SampleIndexRange) -> SampleIndex {
        SampleIndex(self.0 / sample_step.as_usize() * sample_step.as_usize())
    }

    #[track_caller]
    pub fn as_ts(self, sample_duration: Duration) -> Timestamp {
        Timestamp::new(self.0 as u64 * sample_duration.as_u64())
    }

    pub fn as_usize(self) -> usize {
        self.0
    }

    pub fn as_u64(self) -> u64 {
        self.0 as u64
    }

    pub fn try_dec(&mut self) -> Result<(), ()> {
        if self.0 > 0 {
            *self = SampleIndex(self.0 - 1);
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn inc(&mut self) {
        *self = SampleIndex(self.0 + 1);
    }

    #[must_use = "this returns the result of the operation, without modifying the original"]
    pub fn checked_sub(self, rhs: Self) -> Option<SampleIndexRange> {
        self.0.checked_sub(rhs.0).map(SampleIndexRange::new)
    }

    #[must_use = "this returns the result of the operation, without modifying the original"]
    pub fn saturating_sub(self, rhs: Self) -> SampleIndexRange {
        SampleIndexRange::new(self.0.saturating_sub(rhs.0))
    }

    #[must_use = "this returns the result of the operation, without modifying the original"]
    pub fn saturating_sub_range(self, rhs: SampleIndexRange) -> SampleIndex {
        SampleIndex::new(self.0.saturating_sub(rhs.as_usize()))
    }
}

impl From<usize> for SampleIndex {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<u64> for SampleIndex {
    fn from(value: u64) -> Self {
        Self(value as usize)
    }
}

impl From<SampleIndexRange> for SampleIndex {
    fn from(range: SampleIndexRange) -> Self {
        Self(range.as_usize())
    }
}

impl Sub for SampleIndex {
    type Output = SampleIndexRange;

    #[track_caller]
    fn sub(self, rhs: SampleIndex) -> SampleIndexRange {
        SampleIndexRange::new(self.0 - rhs.0)
    }
}

impl Add<SampleIndexRange> for SampleIndex {
    type Output = SampleIndex;

    #[track_caller]
    fn add(self, rhs: SampleIndexRange) -> SampleIndex {
        SampleIndex(self.0 + rhs.as_usize())
    }
}

impl AddAssign<SampleIndexRange> for SampleIndex {
    #[track_caller]
    fn add_assign(&mut self, rhs: SampleIndexRange) {
        *self = SampleIndex(self.0 + rhs.as_usize());
    }
}

impl Sub<SampleIndexRange> for SampleIndex {
    type Output = SampleIndex;

    #[track_caller]
    fn sub(self, rhs: SampleIndexRange) -> SampleIndex {
        SampleIndex(self.0 - rhs.as_usize())
    }
}

impl PartialOrd<SampleIndexRange> for SampleIndex {
    fn partial_cmp(&self, rhs: &SampleIndexRange) -> Option<Ordering> {
        Some(self.0.cmp(&rhs.as_usize()))
    }
}

impl PartialEq<SampleIndexRange> for SampleIndex {
    fn eq(&self, rhs: &SampleIndexRange) -> bool {
        self.0 == rhs.as_usize()
    }
}

impl fmt::Display for SampleIndex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "idx {}", self.0)
    }
}
