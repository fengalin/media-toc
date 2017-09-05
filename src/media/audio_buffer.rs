extern crate gstreamer as gst;
extern crate gstreamer_app as gst_app;

extern crate byte_slice_cast;
use byte_slice_cast::AsSliceOf;

use std::i16;

use std::collections::vec_deque::VecDeque;

use std::sync::{Arc, Mutex};

use super::{SamplesExtractor, WaveformBuffer};

pub struct AudioBuffer {
    capacity: usize,
    pub sample_duration: f64,
    channels: usize,
    drain_size: usize,

    pub samples_offset: usize,
    waveform_samples_offset: usize,
    pub samples: VecDeque<f64>,

    waveform_buffer_mtx: Arc<Mutex<Option<WaveformBuffer>>>,
    second_waveform_buffer: Option<WaveformBuffer>,
}

impl AudioBuffer {
    pub fn new(
        caps: &gst::Caps,
        size_duration: u64,
        waveform_buffer_mtx: Arc<Mutex<Option<WaveformBuffer>>>,
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

            samples_offset: 0,
            waveform_samples_offset: 0,
            samples: VecDeque::with_capacity(capacity),

            waveform_buffer_mtx: waveform_buffer_mtx,
            second_waveform_buffer: Some(WaveformBuffer::new()),
        }
    }

    pub fn push_gst_sample(&mut self, sample: gst::Sample) {
        let buffer = sample.get_buffer()
            .expect("Couldn't get buffer from audio sample");

        let map = buffer.map_readable().unwrap();
        let incoming_samples = map.as_slice().as_slice_of::<i16>()
            .expect("Couldn't get audio samples as i16");

        if self.samples.len() + incoming_samples.len() > self.capacity
        {   // buffer will reach capacity => drain a chunk of samples
            // only if we have samples in history
            let offset_delta = self.waveform_samples_offset - self.samples_offset;
            if offset_delta > self.drain_size {
                self.samples.drain(..self.drain_size);
                self.samples_offset += self.drain_size;
            }
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

        // prepare the second waveform buffer for rendering
        if !self.samples.is_empty() {
            let mut second_waveform_buffer = self.second_waveform_buffer.take()
                .expect("Second waveform buffer not found");

            second_waveform_buffer.extract_samples(&self);

            // switch buffers
            let waveform_buffer = {
                let mut waveform_buffer_opt = self.waveform_buffer_mtx.lock()
                    .expect("Failed to lock the waveform buffer for switch");
                let waveform_buffer = waveform_buffer_opt.take()
                    .expect("No waveform buffer found while switching buffers");
                *waveform_buffer_opt = Some(second_waveform_buffer);

                waveform_buffer
            };

            // get required sample boundary for next draining
            self.waveform_samples_offset = waveform_buffer.get_sample_offset();

            self.second_waveform_buffer = Some(waveform_buffer);
        }
    }

    // TODO: make an iter in order to avoid index comptutation for each call
    pub fn get_sample(&self, absolute_idx: usize) -> f64 {
        self.samples[absolute_idx - self.samples_offset]
    }

    pub fn handle_eos(&mut self) {
        if !self.samples.is_empty() {
            let mut second_waveform_buffer = self.second_waveform_buffer.take()
                .expect("Second waveform buffer not found");

            second_waveform_buffer.handle_eos();
            second_waveform_buffer.extract_samples(&self);

            // replace buffer
            let mut waveform_buffer_opt = self.waveform_buffer_mtx.lock()
                .expect("Failed to lock the waveform buffer for switch");

            *waveform_buffer_opt = Some(second_waveform_buffer);
        }
    }
}
