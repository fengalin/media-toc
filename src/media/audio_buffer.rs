extern crate gstreamer as gst;

#[cfg(feature = "profiling-audio-buffer")]
use chrono::Utc;

use byte_slice_cast::AsSliceOf;

use std::i16;

use std::collections::vec_deque::VecDeque;

use super::{DoubleSampleExtractor};

pub const SAMPLES_NORM: f64 = 200f64;
const SAMPLES_OFFSET: f64 = SAMPLES_NORM / 2f64;

pub struct AudioBuffer {
    capacity: usize,
    pub sample_duration: f64,
    channels: usize,
    drain_size: usize,

    pub eos: bool,

    segement_first_sample: usize,
    last_buffer_last_sample: usize,
    pub first_sample: usize,
    pub last_sample: usize,
    pub samples: VecDeque<f64>,

    samples_extractor_opt: Option<DoubleSampleExtractor>,
}

impl AudioBuffer {
    pub fn new(
        caps: &gst::Caps,
        size_duration: u64,
        samples_extractor: DoubleSampleExtractor,
    ) -> Self
    {
        let structure = caps.get_structure(0)
            .expect("Couldn't get structure from audio caps");
        let rate = structure.get::<i32>("rate")
            .expect("Couldn't get rate from audio caps");

        // assert_eq!(format, S16);
        // assert_eq!(layout, Interleaved);

        let sample_duration = 1_000_000_000f64 / (rate as f64);
        let capacity = (size_duration as f64 / sample_duration) as usize;

        let drain_size = capacity / 5;

        AudioBuffer {
            capacity: capacity,
            sample_duration: sample_duration,
            channels: structure.get::<i32>("channels")
                .expect("Couldn't get channels from audio sample")
                as usize,
            drain_size: drain_size,

            eos: false,

            segement_first_sample: 0,
            last_buffer_last_sample: 0,
            first_sample: 0,
            last_sample: 0,
            samples: VecDeque::with_capacity(capacity),

            samples_extractor_opt: Some(samples_extractor),
        }
    }

    // Add samples from the GStreamer pipeline to the AudioBuffer
    // This buffer stores the complete set of samples in a time frame
    // in order to be able to represent the audio at any given precision.
    // Samples are stores as f64 suitable for on screen rendering.
    // This implementation also performs a downmix.
    // TODO: make the representation dependent on the actual context
    //       move this as a trait and make the sample normalization
    //       part of a concrete object
    // Incoming samples are merged to the existing buffer when possible
    pub fn push_gst_sample(&mut self, sample: gst::Sample) {
        #[cfg(feature = "profiling-audio-buffer")]
        let start = Utc::now();

        let buffer = sample.get_buffer()
            .expect("Couldn't get buffer from audio sample");

        let map = buffer.map_readable().unwrap();
        let incoming_samples = map.as_slice().as_slice_of::<i16>()
            .expect("Couldn't get audio samples as i16");

        #[cfg(feature = "profiling-audio-buffer")]
        let before_drain = Utc::now();

        self.eos = false;

        let segment_first_sample = (
            sample.get_segment().unwrap().get_start() as f64 / self.sample_duration
        ) as usize;
        let buffer_sample_len = incoming_samples.len() / self.channels;

        // Identify conditions for this incoming buffer:
        // 1. Incoming buffer fits at the end of current container.
        // 2. Incoming buffer is already contained within stored samples.
        //    Nothing to do.
        // 3. Incoming buffer overlaps with stored samples at the end.
        // 4. Incoming buffer overlaps with stored samples at the begining.
        //    Note: this changes the first sample and requires to extend
        //    the internal container from the begining.
        // 5. Incoming buffer doesn't overlap with current buffer. In order
        //    not to let gaps between samples, the internal container is
        //    cleared first.
        // 6. The internal container is empty, import incoming buffer
        //    completely.
        let (
            first_sample_changed,
            first_sample_to_add_rel,
            last_sample_to_add_rel
        ) =
            if !self.samples.is_empty()
            {   // not initializing
                let first_sample =
                    if segment_first_sample == self.segement_first_sample {
                        // receiving next buffer in the same segment
                        self.last_buffer_last_sample
                    } else {
                        // different segment (done seeking)
                        self.segement_first_sample = segment_first_sample;
                        self.last_buffer_last_sample = segment_first_sample;
                        segment_first_sample
                    };
                let last_sample = first_sample + buffer_sample_len;

                if first_sample == self.last_sample {
                    // 1. append incoming buffer to the end of internal storage
                    //println!("AudioBuffer appending full incoming buffer to the end");
                    // self.first_sample unchanged
                    self.last_sample = last_sample;
                    self.last_buffer_last_sample = last_sample;
                    (false, 0, buffer_sample_len)
                } else if first_sample >= self.first_sample
                && last_sample <= self.last_sample {
                    // 2. incoming buffer included in current container
                    self.last_buffer_last_sample = last_sample;
                    (false, 0, 0)
                } else if first_sample > self.first_sample
                && first_sample < self.last_sample
                {   // 3. can append [self.last_sample, last_sample] to the end
                    // self.first_sample unchanged
                    let previous_last_sample = self.last_sample;
                    self.last_sample = last_sample;
                    // self.first_pts unchanged
                    self.last_buffer_last_sample = last_sample;
                    (false, previous_last_sample - segment_first_sample, buffer_sample_len)
                }
                else if last_sample < self.last_sample
                && last_sample > self.first_sample
                {   // 4. can insert [first_sample, self.first_sample] to the begining
                    let last_sample_to_add = self.first_sample;
                    self.first_sample = first_sample;
                    // self.last_sample unchanged
                    self.last_buffer_last_sample = last_sample;
                    (true, 0, last_sample_to_add - segment_first_sample)
                } else {
                    // 5. can't merge with previous buffer
                    self.samples.clear();
                    self.first_sample = first_sample;
                    self.last_sample = last_sample;
                    self.last_buffer_last_sample = last_sample;
                    (true, 0, buffer_sample_len)
                }
            } else {
                // 6. initializing
                self.segement_first_sample = segment_first_sample;
                self.first_sample = segment_first_sample;
                self.last_sample = segment_first_sample + buffer_sample_len;
                self.last_buffer_last_sample = self.last_sample;
                (true, 0, buffer_sample_len)
            };

        // TODO rehabilitate drain...
        /*} else if self.samples.len() + incoming_samples.len() > self.capacity
            && self.samples_extractor_opt.as_ref().unwrap().get_first_sample()
                > self.first_sample + self.drain_size
        {   // buffer will reach capacity => drain a chunk of samples
            // only if we have samples in history
            // TODO: it could be worse testing truncate instead
            // (this would require reversing the buffer alimentation
            // and iteration)
            self.samples.drain(..self.drain_size);
            self.first_sample += self.drain_size;
            false*/

        #[cfg(feature = "profiling-audio-buffer")]
        let before_storage = Utc::now();

        // normalize samples in range 0f64..SAMPLES_NORM ready to render
        // TODO: this depends on the actual context (Waveform rendering)
        // do this in a concrete implementation

        // FIXME: use gstreamer downmix
        // FIXME: select the channels using the position info
        // if more than 2 channels,
        // Use 75% for first 2 channels (assuming front left and front right)
        // Use 25% for the others
        if last_sample_to_add_rel > 0 {
            let (front_norm_factor, others_norm_factor, front_channels) =
                if self.channels > 2 {
                    (
                        0.75f64 / 2f64 / f64::from(i16::MAX) * SAMPLES_NORM / 2f64,
                        0.25f64 / ((self.channels - 2) as f64) / f64::from(i16::MAX) * SAMPLES_NORM / 2f64,
                        2
                    )
                } else {
                    (
                        1f64 / (self.channels as f64) / f64::from(i16::MAX) * SAMPLES_OFFSET,
                        0f64,
                        self.channels
                    )
                };

            // Update container using the conditions identified above
            if !first_sample_changed || self.samples.is_empty()
            {   // samples can be push back to the container
                let mut norm_sample;
                let mut index = first_sample_to_add_rel * self.channels;
                let last = last_sample_to_add_rel * self.channels;
                while index < last {
                    norm_sample = 0f64;

                    for _ in 0..front_channels {
                        norm_sample += f64::from(incoming_samples[index]) * front_norm_factor;
                        index += 1;
                    }
                    for _ in front_channels..self.channels {
                        norm_sample += f64::from(incoming_samples[index]) * others_norm_factor;
                        index += 1;
                    }
                    self.samples.push_back(SAMPLES_OFFSET - norm_sample);
                };
            } else
            {   // samples must be inserted at the begining
                // => push front in reverse order
                let mut norm_sample;
                let mut index = last_sample_to_add_rel * self.channels - 1;
                let first = first_sample_to_add_rel * self.channels;
                while index >= first {
                    norm_sample = 0f64;

                    for _ in front_channels..self.channels {
                        norm_sample += f64::from(incoming_samples[index]) * others_norm_factor;
                        index -= 1;
                    }
                    for _ in 0..front_channels {
                        norm_sample += f64::from(incoming_samples[index]) * front_norm_factor;
                        index -= 1;
                    }
                    self.samples.push_front(SAMPLES_OFFSET - norm_sample);
                };
            }
        }

        // Trigger specific buffer extraction
        #[cfg(feature = "profiling-audio-buffer")]
        let before_extract = Utc::now();

        if !self.samples.is_empty() {
            let mut samples_extractor = self.samples_extractor_opt.take().unwrap();
            samples_extractor.extract_samples(self);
            self.samples_extractor_opt = Some(samples_extractor);
        }

        #[cfg(feature = "profiling-audio-buffer")]
        let end = Utc::now();

        #[cfg(feature = "profiling-audio-buffer")]
        println!("audio-buffer,{},{},{},{},{}",
            start.time().format("%H:%M:%S%.6f"),
            before_drain.time().format("%H:%M:%S%.6f"),
            before_storage.time().format("%H:%M:%S%.6f"),
            before_extract.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
        );
    }

    pub fn iter(&self, first: usize, last: usize, step: usize) -> Iter {
        if first < self.first_sample {
            println!("iter {}, {}", first, self.first_sample);
        }
        assert!(first >= self.first_sample);
        let last = if last > first { last } else { first };
        Iter::new(self, first, last, step)
    }

    pub fn handle_eos(&mut self) {
        if !self.samples.is_empty() {
            self.eos = true;

            let mut samples_extractor = self.samples_extractor_opt.take().unwrap();
            samples_extractor.extract_samples(self);
            self.samples_extractor_opt = Some(samples_extractor);
        }
    }
}

pub struct Iter<'a> {
    buffer: &'a AudioBuffer,
    idx: usize,
    last: usize,
    step: usize,
}

impl<'a> Iter<'a> {
    fn new(buffer: &'a AudioBuffer, first: usize, last: usize, step: usize) -> Iter<'a> {
        Iter {
            buffer: buffer,
            idx: first - buffer.first_sample,
            last: buffer.samples.len().min(last - buffer.first_sample),
            step: step,
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a f64;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.last {
            return None;
        }

        let item = self.buffer.samples.get(self.idx);
        self.idx += self.step;

        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.idx >= self.last {
            return (0, Some(0));
        }

        let remaining = (self.last - self.idx) / self.step;

        (remaining, Some(remaining))
    }
}
