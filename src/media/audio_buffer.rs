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
    sample_duration_u: u64,
    channels: usize,
    drain_size: usize,

    pub eos: bool,

    pub samples_offset: usize,
    pub last_sample: usize,
    first_pts: u64,
    last_pts: u64,
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

        let sample_duration_u = 1_000_000_000 / (rate as u64);
        let capacity = (size_duration / sample_duration_u) as usize;

        let drain_size = capacity / 5;

        AudioBuffer {
            capacity: capacity,
            sample_duration: sample_duration_u as f64,
            sample_duration_u: sample_duration_u,
            channels: structure.get::<i32>("channels")
                .expect("Couldn't get channels from audio sample")
                as usize,
            drain_size: drain_size,

            eos: false,

            samples_offset: 0,
            last_sample: 0,
            first_pts: 0,
            last_pts: 0,
            samples: VecDeque::with_capacity(capacity),

            samples_extractor_opt: Some(samples_extractor),
        }
    }

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

        let first_pts = buffer.get_pts() as u64;

        // Note: need to take a margin with last_pts comparison as streams
        // tend to shift buffers back and forth
        if first_pts < self.first_pts || first_pts > self.last_pts + 700_000
        || self.samples.is_empty() {
            // seeking or initializing
            self.samples.clear();
            self.samples_offset = (first_pts / self.sample_duration_u) as usize;
            self.last_sample = self.samples_offset;
            self.first_pts = first_pts;
        } /*else if self.samples.len() + incoming_samples.len() > self.capacity
            && self.samples_extractor_opt.as_ref().unwrap().samples_offset
                > self.samples_offset + self.drain_size
        {   // buffer will reach capacity => drain a chunk of samples
            // only if we have samples in history
            // TODO: it could be worse testing truncate instead
            // (this would require reversing the buffer alimentation
            // and iteration)
            self.samples.drain(..self.drain_size);
            self.samples_offset += self.drain_size;
        }*/

        #[cfg(feature = "profiling-audio-buffer")]
        let before_storage = Utc::now();

        // normalize samples in range 0f64..1f64 ready to render

        // FIXME: use gstreamer downmix
        // FIXME: select the channels using the position info
        // if more than 2 channels,
        // Use 75% for first 2 channels (assumeing front left and front right)
        // Use 25% for the others
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

        let mut norm_sample;
        let mut index = 0;
        while index < incoming_samples.len() {
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

        self.last_sample += incoming_samples.len();

        // resync first_pts for each buffer because each stream has its own
        // rounding strategy
        self.last_pts = first_pts + buffer.get_duration();

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
        println!("audio-buffer,{},{},{},{},{},{}",
            start.time().format("%H:%M:%S%.6f"),
            before_drain.time().format("%H:%M:%S%.6f"),
            before_storage.time().format("%H:%M:%S%.6f"),
            before_extract.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
            index
        );
    }

    pub fn iter(&self, first: usize, last: usize, step: usize) -> Iter {
        if first < self.samples_offset {
            println!("iter {}, {}", first, self.samples_offset);
        }
        assert!(first >= self.samples_offset);
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
            idx: first - buffer.samples_offset,
            last: buffer.samples.len().min(last - buffer.samples_offset),
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
