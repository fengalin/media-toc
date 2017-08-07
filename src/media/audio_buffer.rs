extern crate byteorder;
use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};

extern crate gstreamer as gst;
use gstreamer::PadExt;

use std::clone::Clone;

use std::io::Cursor;

use std::ops::{Deref, DerefMut};

use super::Timestamp;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SampleFormat {
    F32LE,
    F64LE,
    I16LE,
    I32LE,
    I64LE,
    U8,
    Unknown,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SampleLayout {
    Interleaved,
    Unknown,
}

#[derive(Copy, Clone, Debug)]
pub struct AudioCaps {
    pub sample_format: SampleFormat,
    pub layout: SampleLayout,
    pub rate: usize,
    pub sample_duration: f64,
    pub channels: usize,
}

impl AudioCaps {
    pub fn new() -> Self {
        AudioCaps {
            sample_format: SampleFormat::Unknown,
            layout: SampleLayout::Interleaved,
            rate: 0,
            sample_duration: 0f64,
            channels: 0,
        }
    }

    pub fn from_sink_pad(sink_pad: &gst::Pad) -> Self {
        let caps = sink_pad.get_current_caps()
            .expect("Couldn't get caps for audio stream");
        let structure = caps.iter().next()
            .expect("AudioCaps::from_gst_caps: No caps found");

        println!("\nAudio sink caps:\n\t{:?}", structure);

        let mut ac = AudioCaps::new();

        let format = structure.get::<String>("format")
            .expect("AudioCaps::from_gst_caps: Couldn't get sample format for audio stream");
        ac.sample_format = if format == "F32LE" {
            SampleFormat::F32LE
        } else if format == "F64LE" {
            SampleFormat::F64LE
        } else if format == "S16LE" {
            SampleFormat::I16LE
        } else if format == "S32LE" {
            SampleFormat::I32LE
        } else if format == "S64LE" {
            SampleFormat::I64LE
        } else if format == "U8" {
            SampleFormat::U8
        } else {
            panic!("AudioCaps::from_gst_caps: Unknown sample format: {}", format);
        };

        let layout = structure.get::<String>("layout")
            .expect("AudioCaps::from_gst_caps: Couldn't get sample layout for audio stream");
        ac.layout = if layout == "interleaved" {
            SampleLayout::Interleaved
        } else {
            panic!("AudioCaps::from_gst_caps: Unknown sample layout: {}", layout);
        };

        ac.rate = structure.get::<i32>("rate")
            .expect("AudioCaps::from_gst_caps: Couldn't get sample rate for audio stream")
            as usize;
        ac.sample_duration = 1_000_000_000f64 / ac.rate as f64;

        ac.channels = structure.get::<i32>("channels")
            .expect("AudioCaps::from_gst_caps: Couldn't get sample channels for audio stream")
            as usize;

        ac
    }
}


pub struct AudioChannel {
    pub id: usize,
    samples: Vec<f64>,
}

impl AudioChannel {
    pub fn new(id: usize) -> Self {
        AudioChannel {
            id: id,
            samples: Vec::new(),
        }
    }

    pub fn get_id(&self) -> usize {
        self.id
    }
}

impl Deref for AudioChannel {
	type Target = Vec<f64>;

	fn deref(&self) -> &Self::Target {
		&self.samples
	}
}

impl DerefMut for AudioChannel {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.samples
	}
}

pub struct AudioBuffer {
    pub caps: AudioCaps,
    pub pts: usize,
    pub duration: usize,
    pub sample_offset: usize,
    pub samples_nb: usize,
    pub channels: Vec<AudioChannel>,
}

impl AudioBuffer {
    pub fn from_gst_buffer(caps: &AudioCaps, buffer: &gst::Buffer) -> Self {
        let mut this = AudioBuffer {
            caps: caps.clone(),
            pts: buffer.get_pts() as usize,
            duration: buffer.get_duration() as usize,
            sample_offset: (buffer.get_pts() as f64 / caps.sample_duration) as usize,
            samples_nb: (buffer.get_duration() as f64 / caps.sample_duration) as usize,
            channels: Vec::with_capacity(caps.channels),
        };

        for channel in 0..this.caps.channels {
            this.channels.push(AudioChannel::new(channel));
        }

        let map = buffer.map_read().unwrap();
        let data = map.as_slice();

        let mut keep_going = true;
        let mut data_reader = Cursor::new(data);

        assert!(this.caps.layout == SampleLayout::Interleaved);
        while keep_going {
            for channel in 0..this.caps.channels {
                let norm_sample = match this.caps.sample_format {
                    SampleFormat::F32LE => {
                        data_reader.read_f32::<LittleEndian>().map(|v| v as f64)
                    },
                    SampleFormat::F64LE => {
                        data_reader.read_f64::<LittleEndian>()
                    },
                    SampleFormat::I16LE => {
                        data_reader.read_i16::<LittleEndian>().map(|v|
                            v as f64 / ::std::i16::MAX as f64
                        )
                    },
                    SampleFormat::I32LE => {
                        data_reader.read_i32::<LittleEndian>().map(|v|
                            v as f64 / ::std::i32::MAX as f64
                        )
                    },
                    SampleFormat::I64LE => {
                        data_reader.read_i64::<LittleEndian>().map(|v|
                            v as f64 / ::std::i64::MAX as f64
                        )
                    },
                    SampleFormat::U8 => {
                        data_reader.read_u8().map(|v|
                            (v as f64 - ::std::i8::MAX as f64) / ::std::i8::MAX as f64
                        )
                    },
                    _ => panic!("never happens"), // FIXME: use proper assert
                };

                match norm_sample {
                    Ok(norm_sample) => this.channels[channel].push(norm_sample),
                    Err(_) => keep_going = false,
                }
            }
        }

        this
    }
}
