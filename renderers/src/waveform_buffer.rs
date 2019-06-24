use log::debug;

use media::{AudioBuffer, SampleIndex, SampleIndexRange, INLINE_CHANNELS};

type DisplaySample = f64;

// Translating samples in the negative range when scaling for display
// improves the rendering bench by 10%
const SAMPLE_AMPLITUDE: i32 = std::i16::MAX as i32;
const SAMPLE_RANGE: DisplaySample = (2 as DisplaySample) * (std::i16::MIN as DisplaySample);

// The `WaveformBuffer` contains c channels of n samples each, usable by `WaveformTracer`
const INIT_SAMPLES_PER_CHANNELS: usize = 1500;

#[derive(Default)]
pub struct WaveformBuffer {
    id: usize,

    buffer: Vec<DisplaySample>,
    channels: usize,
    samples_per_channel: usize,
    pub(super) contains_eos: bool,
    pub(super) lower: SampleIndex,
    pub(super) upper: SampleIndex,

    pub(super) force_extraction: bool,
    height: DisplaySample,
}

impl WaveformBuffer {
    pub fn new(id: usize) -> Self {
        WaveformBuffer {
            id,
            buffer: Vec::with_capacity(INLINE_CHANNELS * INIT_SAMPLES_PER_CHANNELS),
            ..WaveformBuffer::default()
        }
    }

    pub fn reset(&mut self) {
        debug!("{}_reset", self.id);

        self.buffer.clear();
        self.channels = 0;
        self.samples_per_channel = 0;
        self.contains_eos = false;
        self.lower = SampleIndex::default();
        self.upper = SampleIndex::default();

        self.force_extraction = false;
    }

    pub fn update_height(&mut self, height: i32) {
        self.height = DisplaySample::from(height);
    }

    pub fn update_from_other(&mut self, other: &WaveformBuffer) {
        debug!("{}_update_from_other", self.id);

        self.height = other.height;
    }

    pub fn iter(&self) -> ChannelsIter {
        ChannelsIter::new(self)
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

        self.buffer.clear();
        self.channels = audio_buffer.channels.min(INLINE_CHANNELS);
        let scale = self.height / SAMPLE_RANGE;

        for channel in 0..self.channels {
            audio_buffer
                .try_iter(lower, upper, channel, sample_step)
                .unwrap_or_else(|err| {
                    panic!("{}_extract_waveform_DisplaySamples: {}", self.id, err)
                })
                .for_each(|sample| {
                    self.buffer.push(
                        DisplaySample::from(i32::from(sample.as_i16()) - SAMPLE_AMPLITUDE) * scale,
                    );
                });
        }

        self.samples_per_channel = (upper - lower).get_step_range(sample_step);
        self.lower = lower;
        self.upper = upper;
    }
}

pub struct ChannelsIter<'buffer> {
    buffer: &'buffer Vec<DisplaySample>,
    samples_per_channel: usize,
    lower: usize,
    upper: usize,
}
impl<'buffer> ChannelsIter<'buffer> {
    fn new(wf_buffer: &'buffer WaveformBuffer) -> Self {
        ChannelsIter {
            buffer: &wf_buffer.buffer,
            samples_per_channel: wf_buffer.samples_per_channel,
            lower: 0,
            upper: wf_buffer.channels * wf_buffer.samples_per_channel,
        }
    }
}

impl<'buffer> Iterator for ChannelsIter<'buffer> {
    type Item = &'buffer [DisplaySample];

    fn next(&mut self) -> Option<Self::Item> {
        if self.lower < self.upper {
            let current_upper = self.lower + self.samples_per_channel;
            let item = &self.buffer[self.lower..current_upper];
            self.lower = current_upper;

            Some(item)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.lower >= self.upper {
            return (0, Some(0));
        }

        let remaining = (self.upper - self.lower) / self.samples_per_channel;

        (remaining, Some(remaining))
    }
}

impl<'buffer> ExactSizeIterator for ChannelsIter<'buffer> {}
