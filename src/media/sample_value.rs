use std::ops::Mul;

#[derive(Clone, Copy, Default, Debug)]
pub struct SampleValue(f64);

impl SampleValue {
    pub fn as_f64(&self) -> f64 {
        self.0
    }
}

impl Mul for SampleValue {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        SampleValue(self.0 * rhs.0)
    }
}

impl From<f64> for SampleValue {
    fn from(inner: f64) -> Self {
        Self(inner)
    }
}

impl From<i32> for SampleValue {
    fn from(inner: i32) -> Self {
        Self(f64::from(inner))
    }
}

#[cfg(test)]
use std::cmp::{Eq, Ordering};

#[cfg(test)]
impl Ord for SampleValue {
    fn cmp(&self, other: &Self) -> Ordering {
        let delta = self.0 - other.0;
        let threshold = 0.000_000_000_1f64 * self.0;
        if delta.abs() < threshold {
            Ordering::Equal
        } else if delta < 0f64 {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
}

#[cfg(test)]
impl PartialOrd for SampleValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
impl PartialEq for SampleValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
#[cfg(test)]
impl Eq for SampleValue {}

#[cfg(test)]
#[macro_export]
macro_rules! i16_to_sample_value(
    ($value:expr) => {
        SampleValue::from(1f64 + f64::from($value) / f64::from(std::i16::MIN))
    };
);
