extern crate gstreamer as gst;
extern crate gstreamer_app as gst_app;

extern crate byte_slice_cast;
use byte_slice_cast::AsSliceOf;

use std::i16;

use std::collections::vec_deque::VecDeque;

use std::sync::{Arc, Mutex};

use super::WaveformBuffer;

pub struct AudioBuffer {
    pts_offset: u64,
    capacity: usize,
    pub sample_duration: u64,
    channels: usize,
    drain_size: usize,
    drain_duration: u64,

    pub samples_offset: usize,
    pub first_pts: u64,
    pub last_pts: u64,
    pub duration: u64,
    pub samples: VecDeque<f64>,

    waveform_buffer_mtx: Arc<Mutex<Option<WaveformBuffer>>>,
    second_waveform_buffer: Option<WaveformBuffer>,
}

impl AudioBuffer {
    pub fn new(
        caps: &gst::Caps,
        pts_offset: u64,
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

        let sample_duration = 1_000_000_000 / (rate as u64);
        let capacity = (size_duration / sample_duration) as usize;

        let drain_size = capacity / 5;

        AudioBuffer {
            pts_offset: pts_offset,
            capacity: capacity,
            sample_duration: sample_duration,
            channels: structure.get::<i32>("channels")
                .expect("Couldn't get channels from audio sample")
                as usize,
            drain_size: drain_size,
            drain_duration: (drain_size as u64) * sample_duration,

            samples_offset: 0,
            first_pts: 0,
            last_pts: 0,
            duration: 0,
            samples: VecDeque::with_capacity(capacity),

            waveform_buffer_mtx: waveform_buffer_mtx,
            second_waveform_buffer: Some(WaveformBuffer::new()),
        }
    }

    pub fn push_gst_sample(&mut self, sample: gst::Sample) {
        let buffer = sample.get_buffer()
            .expect("Couldn't get buffer from audio sample");

        let pts = buffer.get_pts();
        if self.samples.is_empty() {
            self.first_pts = pts;
            self.samples_offset = (self.first_pts / self.sample_duration) as usize;
            self.last_pts = self.first_pts;
        }

        let map = buffer.map_readable().unwrap();
        let incoming_samples = map.as_slice().as_slice_of::<i16>()
            .expect("Couldn't get audio samples as i16");

        // Use outputs from the double buffer preparation to decide
        // what to drain
        if self.samples.len() + incoming_samples.len() > self.capacity
            && pts > self.pts_offset
        {   // buffer will reach capacity => drain a chunk of samples
            // only if we have samples in history
            let pts = pts - self.pts_offset;
            if pts - self.first_pts > 2 * self.drain_duration {
                self.samples.drain(..self.drain_size);
                self.samples_offset += self.drain_size;
                self.first_pts += self.drain_duration;
                self.duration -= self.drain_duration;
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

        let duration = buffer.get_duration();
        self.last_pts += duration;
        self.duration += duration;

        // FIXME: need to detect the last buffer in order to fill the
        // waveform completly since we won't come back here after that

        // prepare the second waveform buffer for rendering
        if self.duration > 0 && pts > self.pts_offset {
            let mut second_waveform_buffer = self.second_waveform_buffer.take()
                .expect("Second waveform buffer not found");

            second_waveform_buffer.set_position(pts - self.pts_offset);
            second_waveform_buffer.update_samples(&self);

            // switch buffers
            let waveform_buffer = {
                let mut waveform_buffer_opt = self.waveform_buffer_mtx.lock()
                    .expect("Failed to lock the waveform buffer for switch");
                let waveform_buffer = waveform_buffer_opt.take()
                    .expect("No waveform buffer found while switch buffers");
                *waveform_buffer_opt = Some(second_waveform_buffer);

                waveform_buffer
            };

            // update buffer with latest samples
            //waveform_buffer.update_samples(&self);

            self.second_waveform_buffer = Some(waveform_buffer);
        }
    }
}
