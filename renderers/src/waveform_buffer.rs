use log::debug;
use smallvec::SmallVec;

use std::ops::Deref;

use media::{AudioBuffer, SampleIndex, SampleIndexRange, INLINE_CHANNELS};

// The `WaveformBuffer` contains c channels of n samples each
type ChannelBuffer = Vec<f64>;
type ChannelsBuffer = SmallVec<[ChannelBuffer; INLINE_CHANNELS]>;

#[derive(Default)]
pub struct WaveformBuffer {
    id: usize,

    channels: ChannelsBuffer,
    pub(super) contains_eos: bool,
    pub(super) lower: SampleIndex,
    pub(super) upper: SampleIndex,

    pub(super) force_extraction: bool,
    pub(super) sample_value_factor: f64,
}

impl WaveformBuffer {
    pub fn new(id: usize) -> Self {
        WaveformBuffer {
            id,
            ..WaveformBuffer::default()
        }
    }

    pub fn reset(&mut self) {
        debug!("{}_reset", self.id);

        self.channels.clear();
        self.contains_eos = false;
        self.lower = SampleIndex::default();
        self.upper = SampleIndex::default();

        self.force_extraction = false;
    }

    pub fn update_from_other(&mut self, other: &WaveformBuffer) {
        debug!("{}_update_from_other", self.id);

        self.sample_value_factor = other.sample_value_factor;
    }

    pub fn extract(
        &mut self,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
        sample_step: SampleIndexRange,
    ) {
        // Align requested lower and upper sample bounds in order to keep
        // a steady offset between redraws. This allows using the same samples
        // for a given req_step_duration and avoiding flickering
        // between redraws.
        let mut lower = lower.get_aligned(sample_step);
        if lower < audio_buffer.lower {
            // first sample might be smaller than audio_buffer.lower
            // due to alignement on sample_step
            lower += sample_step;
        }

        // When audio_buffer contains eof, we won't be called again => extract all we can get
        self.contains_eos = audio_buffer.contains_eos();
        let upper = if !self.contains_eos {
            upper.get_aligned(sample_step)
        } else {
            audio_buffer.upper.get_aligned(sample_step)
        };

        if upper < lower + sample_step {
            debug!(
                "{}_extract range [{}, {}] too small for sample_step: {}",
                self.id, lower, upper, sample_step,
            );
            return;
        }

        if !self.force_extraction
            && !self.contains_eos
            && upper <= self.upper
            && lower >= self.lower
        {
            // target extraction fits in previous extraction
            return;
        }

        self.channels.clear();
        for channel in 0..audio_buffer.channels {
            // TODO: try this with crate `faster`
            self.channels.push(
                audio_buffer
                    .try_iter(lower, upper, channel, sample_step)
                    .unwrap_or_else(|err| panic!("{}_extract_waveform_samples: {}", self.id, err))
                    .map(|channel_value| {
                        f64::from(i32::from(channel_value.as_i16()) - i32::from(std::i16::MAX))
                            * self.sample_value_factor
                    })
                    .collect(),
            );
        }

        self.lower = lower;
        self.upper = upper;
    }
}

impl Deref for WaveformBuffer {
    type Target = ChannelsBuffer;

    fn deref(&self) -> &Self::Target {
        &self.channels
    }
}
