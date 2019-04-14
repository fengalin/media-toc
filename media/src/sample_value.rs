#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub struct SampleValue(i16);

impl SampleValue {
    pub fn as_i16(&self) -> i16 {
        self.0
    }
}

impl From<i16> for SampleValue {
    fn from(value: i16) -> Self {
        Self(value)
    }
}
