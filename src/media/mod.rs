pub mod audio_channel;
pub use self::audio_channel::{AudioChannel, AudioChannelSide, INLINE_CHANNELS};

pub mod audio_buffer;
pub use self::audio_buffer::AudioBuffer;

pub mod dbl_audio_buffer;
pub use self::dbl_audio_buffer::DoubleAudioBuffer;

pub mod playback_pipeline;
pub use self::playback_pipeline::{PlaybackPipeline, QUEUE_SIZE_NS};

pub mod sample_extractor;
pub use self::sample_extractor::SampleExtractor;

pub mod splitter_pipeline;
pub use self::splitter_pipeline::SplitterPipeline;

pub mod toc_setter_pipeline;
pub use self::toc_setter_pipeline::TocSetterPipeline;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybackState {
    Paused,
    Playing,
}

use std::ops::Mul;

#[cfg(test)]
use std::cmp::{Eq, Ordering};

#[derive(Clone, Debug)]
pub enum MediaEvent {
    AsyncDone(PlaybackState),
    Eos,
    FailedToOpenMedia(String),
    FailedToExport(String),
    InitDone,
    MissingPlugin(String),
    ReadyForRefresh,
    StreamsSelected,
}

macro_rules! numeric_types(
    ($type_:ident, $inner_type:ty) => (
        #[derive(Clone, Copy, Default, Debug)]
        pub struct $type_($inner_type);

        impl Mul for $type_ {
            type Output = Self;

            fn mul(self, rhs: Self) -> Self {
                $type_(self.0 * rhs.0)
            }
        }

        impl From<$inner_type> for $type_ {
            fn from(inner: $inner_type) -> Self {
                Self(inner)
            }
        }
    );
);

numeric_types!(SampleValue, f64);
impl SampleValue {
    pub fn as_f64(&self) -> f64 {
        self.0
    }
}

impl From<i32> for SampleValue {
    fn from(inner: i32) -> Self {
        Self(f64::from(inner))
    }
}

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
