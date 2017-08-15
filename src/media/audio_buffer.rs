extern crate byteorder;
use byteorder::{LittleEndian, ReadBytesExt};

extern crate gstreamer as gst;
use gstreamer::PadExt;

extern crate gstreamer_audio as gst_audio;
use gstreamer_audio::AudioFormat;

use std::io::Cursor;

use std::ops::Deref;

pub struct AudioInfo {
    pub info: gst_audio::AudioInfo,
    pub sample_duration: u64,
}

impl AudioInfo {
    pub fn from_sink_pad(sink_pad: &gst::Pad) -> Self {
        let caps = sink_pad.get_current_caps()
            .expect("Couldn't get caps for audio stream");
        let gst_audio_info = gst_audio::AudioInfo::from_caps(&caps)
            .expect("Couldn't get audio info for audio stream");

        AudioInfo {
            sample_duration: 1_000_000_000 / (gst_audio_info.rate() as u64),
            info: gst_audio_info,
        }
    }
}

impl Deref for AudioInfo {
	type Target = gst_audio::AudioInfo;

	fn deref(&self) -> &Self::Target {
		&self.info
	}
}

pub struct AudioBuffer {
    pub sample_duration: u64,
    pub pts: u64,
    pub duration: u64,
    pub samples: Vec<f64>,
}

impl AudioBuffer {
    pub fn from_gst_buffer(info: &AudioInfo, buffer: &gst::Buffer) -> Self {
        // TODO: don't compute sample duration every time
        let duration = buffer.get_duration();
        let sample_nb = (duration / info.sample_duration) as u32;
        let mut this = AudioBuffer {
            sample_duration: info.sample_duration,
            pts: buffer.get_pts(),
            duration: duration,
            samples: Vec::with_capacity((info.channels() * sample_nb) as usize),
        };

        assert_eq!(info.layout(), gst_audio::AudioLayout::Interleaved);

        let map = buffer.map_readable().unwrap();
        let data = map.as_slice();

        let mut data_reader = Cursor::new(data);
        let channels_f = info.channels() as f64;
        let mut keep_going = true;
        while keep_going {
            let mut mono_sample = 0f64;
            for _ in 0..info.channels() {
                let norm_sample = match info.format() {
                    AudioFormat::F32le => {
                        data_reader.read_f32::<LittleEndian>().map(|v| v as f64)
                    },
                    AudioFormat::F64le => {
                        data_reader.read_f64::<LittleEndian>()
                    },
                    AudioFormat::S16le => {
                        data_reader.read_i16::<LittleEndian>().map(|v|
                            v as f64 / ::std::i16::MAX as f64
                        )
                    },
                    AudioFormat::S32le => {
                        data_reader.read_i32::<LittleEndian>().map(|v|
                            v as f64 / ::std::i32::MAX as f64
                        )
                    },
                    AudioFormat::U8 => {
                        data_reader.read_u8().map(|v|
                            (v as f64 - ::std::i8::MAX as f64) / ::std::i8::MAX as f64
                        )
                    },
                    _ => panic!("never happens"), // FIXME: use proper assert
                };

                match norm_sample {
                    Ok(norm_sample) => mono_sample += norm_sample,
                    Err(_) => {
                        keep_going = false;
                        break;
                    },
                }
            }

            if keep_going {
                this.samples.push(1f64 - (mono_sample / channels_f));
            }
        }

        this
    }
}
