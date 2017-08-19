extern crate gstreamer as gst;
extern crate gstreamer_app as gst_app;

extern crate byte_slice_cast;
use byte_slice_cast::AsSliceOf;

use std::i16;

use std::collections::vec_deque::VecDeque;

pub struct AudioBuffer {
    pub capacity: usize,
    pub sample_duration: u64,
    pub channels: usize,
    pub drain_size: usize,
    pub drain_duration: u64,

    pub first_sample_offset: usize,
    pub first_pts: u64,
    pub current_pts: u64,
    pub current_pts_relative: u64,
    pub last_pts: u64,
    pub duration: u64,
    pub samples: VecDeque<f64>,
}

impl AudioBuffer {
    pub fn new() -> Self {
        AudioBuffer {
            capacity: 0,
            sample_duration: 0,
            channels: 0,
            drain_size: 1,
            drain_duration: 0,

            first_sample_offset: 0,
            first_pts: 0,
            current_pts: 0,
            current_pts_relative: 0,
            last_pts: 0,
            duration: 0,
            samples: VecDeque::new(),
        }
    }

    pub fn initialize(&mut self, caps: &gst::Caps, size_duration: u64) {
        let structure = caps.get_structure(0)
            .expect("Couldn't get structure from audio caps");
        let rate = structure.get::<i32>("rate")
            .expect("Couldn't get rate from audio caps");

        // assert_eq!(format, S16);
        // assert_eq!(layout, Interleaved);

        let sample_duration = 1_000_000_000 / (rate as u64);
        let capacity = (size_duration / sample_duration) as usize;

        let drain_size = capacity / 5;

        self.capacity = capacity;
        self.sample_duration = sample_duration;
        self.channels =structure.get::<i32>("channels")
            .expect("Couldn't get channels from audio sample")
            as usize;
        self.drain_size = drain_size;
        self.drain_duration = (drain_size as u64) * sample_duration;

        self.first_sample_offset = 0;
        self.first_pts = 0;
        self.current_pts = 0;
        self.current_pts_relative = 0;
        self.last_pts = 0;
        self.duration = 0;

        let current_capacity = self.samples.capacity();
        if current_capacity < capacity {
            self.samples.reserve(capacity - current_capacity);
        }
    }

    pub fn set_first_pts(&mut self, pts: u64) {
        if self.samples.is_empty() {
            self.first_sample_offset = (pts / self.sample_duration) as usize;
            self.first_pts = pts;
            self.set_pts(pts);
            self.last_pts += pts;
        }
    }

    pub fn set_pts(&mut self, pts: u64) {
        self.current_pts = pts;
        self.current_pts_relative = pts - self.first_pts;
    }

    pub fn push_gst_sample(&mut self, sample: gst::Sample) {
        let buffer = sample.get_buffer()
            .expect("Couldn't get buffer from audio sample");

        let map = buffer.map_readable().unwrap();
        let incoming_samples = map.as_slice().as_slice_of::<i16>()
            .expect("Couldn't get audio samples as i16");

        if self.samples.len() + incoming_samples.len() > self.capacity
            && self.current_pts_relative > 2_000_000_000
        {   // buffer will reach capacity => drain a chunk of samples
            // only if we have 2 sec worse of samples in history
            self.samples.drain(..self.drain_size);
            self.first_sample_offset += self.drain_size;
            self.first_pts += self.drain_duration;
            self.current_pts_relative -= self.drain_duration;
            self.duration -= self.drain_duration;
        }

        // normalize samples in range 0f64..2f64 ready to render

        // FIXME: use gstreamer downmix
        // FIXME: select the channels using the position info
        // if more than 2 channels,
        // Use 75% for first 2 channels (assumes front left and front right)
        // Use 25% for the rest
        let (front_norm_factor, others_norm_factor, front_channels) =
            if self.channels > 2 {
                (
                    0.75f64 / 2f64 / (i16::MAX as f64),
                    0.25f64 / ((self.channels - 2) as f64) / (i16::MAX as f64),
                    2
                )
            } else {
                (
                    1f64 / (self.channels as f64) / (i16::MAX as f64),
                    0f64,
                    self.channels
                )
            };

        let mut norm_sample;
        let mut index = 0;
        while index < incoming_samples.len() {
            norm_sample = 0f64;

            for _ in 0..front_channels {
                norm_sample += incoming_samples[index] as f64 * front_norm_factor;
                index += 1;
            }
            for _ in front_channels..self.channels {
                norm_sample += incoming_samples[index] as f64 * others_norm_factor;
                index += 1;
            }
            self.samples.push_back(1f64 - norm_sample);
        };

        let duration = buffer.get_duration();
        self.last_pts += duration;
        self.duration += duration;
    }
}
