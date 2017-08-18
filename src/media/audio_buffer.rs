extern crate gstreamer as gst;
extern crate gstreamer_app as gst_app;

extern crate byte_slice_cast;
use byte_slice_cast::AsSliceOf;

use std::i16;

pub struct AudioBuffer {
    pub sample_duration: u64,
    pub pts: u64,
    pub duration: u64,
    pub samples: Vec<f64>,
}

impl AudioBuffer {
    pub fn from_gst_buffer(caps: &gst::Caps, buffer: &gst::Buffer) -> Self {
        let structure = caps.get_structure(0)
            .expect("Couldn't get structure from audio sample");
        let rate = structure.get::<i32>("rate")
            .expect("Couldn't get rate from audio sample");
        // FIXME: channels is set in appsink
        let channels = structure.get::<i32>("channels")
            .expect("Couldn't get channels from audio sample")
            as usize;
        let channels_f = channels as f64;

        // assert_eq!(format, S16);

        let sample_duration = 1_000_000_000 / (rate as u64);

        let map = buffer.map_readable().unwrap();
        let data = map.as_slice().as_slice_of::<i16>()
            .expect("Couldn't get audio samples as i16");
        let sample_nb = data.len() / channels;

        let mut this = AudioBuffer {
            sample_duration: sample_duration,
            pts: buffer.get_pts(),
            duration: buffer.get_duration(),
            samples: Vec::with_capacity(sample_nb),
        };

        for index in 0..sample_nb {
            let mut mono_sample = 0f64;
            // FIXME: downmix in the pipeline (maybe done by appsink)
            for channel in 0..channels {
                mono_sample += data[index + channel] as f64;
            }

            this.samples.push(1f64 - (mono_sample / (i16::MAX as f64) / channels_f));
        }

        this
    }

    pub fn from_gst_sample(sample: gst::Sample) -> Self {
        let caps = sample.get_caps()
            .expect("Couldn't get caps from sample");
        let structure = caps.get_structure(0)
            .expect("Couldn't get structure from audio sample");
        let rate = structure.get::<i32>("rate")
            .expect("Couldn't get rate from audio sample");
        // FIXME: channels is set in appsink
        let channels = structure.get::<i32>("channels")
            .expect("Couldn't get channels from audio sample")
            as usize;
        let channels_f = channels as f64;

        let buffer = sample.get_buffer()
            .expect("Couldn't get buffer from audio sample");

        // assert_eq!(format, S16);

        let sample_duration = 1_000_000_000 / (rate as u64);

        let map = buffer.map_readable().unwrap();
        let data = map.as_slice().as_slice_of::<i16>()
            .expect("Couldn't get audio samples as i16");
        let sample_nb = data.len() / channels;

        let mut this = AudioBuffer {
            sample_duration: sample_duration,
            pts: buffer.get_pts(),
            duration: buffer.get_duration(),
            samples: Vec::with_capacity(sample_nb),
        };

        for index in 0..sample_nb {
            let mut mono_sample = 0f64;
            // FIXME: downmix in the pipeline (maybe done by appsink)
            for channel in 0..channels {
                mono_sample += data[index + channel] as f64;
            }

            this.samples.push(1f64 - (mono_sample / (i16::MAX as f64) / channels_f));
        }

        this
    }
}
