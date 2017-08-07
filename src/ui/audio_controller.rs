extern crate byteorder;
use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};

extern crate glib;

extern crate gtk;
use gtk::{Inhibit, WidgetExt};

extern crate cairo;

extern crate gstreamer as gst;
use gstreamer::{BinExt, ElementExt, PadExt};

use std::rc::{Rc, Weak};
use std::cell::RefCell;

use std::collections::vec_deque::{VecDeque};

use std::io::Cursor;

use std::ops::{Deref, DerefMut};

use ::media::{Context, Timestamp};

use super::{MediaController, MediaHandler};

pub enum SampleFormat {
    F32LE,
    F64LE,
    I16LE,
    I32LE,
    I64LE,
    U8,
    Unknown,
}

pub enum SampleLayout {
    Interleaved,
    Unknown,
}

pub struct AudioController {
    media_ctl: MediaController,
    drawingarea: gtk::DrawingArea,

    sample_format: SampleFormat,
    layout: SampleLayout,
    rate: usize,
    channels: usize,
    circ_buffer: VecDeque<gst::Buffer>,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let ac = Rc::new(RefCell::new(AudioController {
            media_ctl: MediaController::new(
                builder.get_object("audio-container").unwrap(),
            ),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            circ_buffer: VecDeque::new(),

            sample_format: SampleFormat::Unknown,
            layout: SampleLayout::Unknown,
            rate: 0,
            channels: 0,
        }));

        {
            let ac_ref = ac.borrow();
            let ac_weak = Rc::downgrade(&ac);
            ac_ref.drawingarea.connect_draw(move |ref drawing_area, ref cairo_ctx| {
                let ac = ac_weak.upgrade()
                    .expect("Main controller is no longer available for select_media");
                return ac.borrow().draw(drawing_area, cairo_ctx);
            });
        }

        ac
    }

    pub fn clear(&mut self) {
        self.circ_buffer.clear();
    }

    pub fn have_buffer(&mut self, buffer: gst::Buffer) {
        // TODO: indexation must use buffer's ts
        self.circ_buffer.push_back(buffer);
    }

    fn draw(&self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) -> Inhibit {
        if self.circ_buffer.len() == 0 {
            return Inhibit(false);
        }

        let sample_dyn = 1024f64;

        let allocation = drawing_area.get_allocation();
        cr.scale(
            allocation.width as f64 / 2048f64, // TODO: compute length using the buffers duration
            allocation.height as f64 / 2f64 / sample_dyn,
        );
        cr.set_line_width(0.5f64);

        // TODO: perform all the conversions (samples, ...) upon buffer reception
        // so that we don't handle all this on each draw
        // and separate channels as it will avoid moving to previous sample position
        // Build a class similar to aligned_image and send it via HaveAudioBuffer
        // this will allow saving one copy

        let mut x = 0f64;
        let mut coordinates = Vec::<(f64, f64)>::with_capacity(self.channels);
        for ref buffer in self.circ_buffer.iter() {
            let map = buffer.map_read().unwrap();
            let data = map.as_slice();
            if data.len() == 0 {
                continue;
            }

            let mut keep_going = true;
            let mut data_reader = Cursor::new(data);
            // FIXME: this is applicable for interleaved only
            while keep_going {
                for channel in 0..self.channels {
                    let norm_sample = match self.sample_format {
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
                        Ok(norm_sample) => {
                            let y = sample_dyn * (1f64 - norm_sample);
                            if x > 0f64 {
                                let colors = vec![(0.8f64, 0.8f64, 0.8f64), (0.8f64, 0f64, 0f64)][channel];
                                cr.set_source_rgb(colors.0, colors.1, colors.2);

                                let (prev_x, prev_y) = coordinates[channel];
                                cr.move_to(prev_x, prev_y);
                                cr.line_to(x, y);

                                // TODO: draw by channel and stroke after each channel
                                cr.stroke();

                                coordinates[channel] = (x, y);
                            } else {
                                coordinates.push((x, y));
                            }
                        },
                        Err(_) => keep_going = false,
                    }
                }

                x += 1f64;

                if x > 2048f64 { keep_going = false; }
            }
        }

        Inhibit(false)
    }
}

impl Deref for AudioController {
	type Target = MediaController;

	fn deref(&self) -> &Self::Target {
		&self.media_ctl
	}
}

impl DerefMut for AudioController {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.media_ctl
	}
}

impl MediaHandler for AudioController {
    fn new_media(&mut self, ctx: &Context) {
        if let Some(audio_sink) = ctx.pipeline.get_by_name("audio_sink") {
            let caps = audio_sink.get_static_pad("sink").unwrap()
                .get_current_caps()
                .expect("Couldn't get caps for audio stream");
            let structure = caps.iter().next()
                .expect("No caps found for audio stream");

            println!("\nAudio sink caps:\n\t{:?}", structure);

            let format = structure.get::<String>("format")
                .expect("Couldn't get sample format for audio stream");
            self.sample_format = if format == "F32LE" {
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
                panic!("Unknown sample format: {}", format);
            };

            let layout = structure.get::<String>("layout")
                .expect("Couldn't get sample layout for audio stream");
            self.layout = if layout == "interleaved" {
                SampleLayout::Interleaved
            } else {
                panic!("Unknown sample layout: {}", layout);
            };

            self.rate = structure.get::<i32>("rate")
                .expect("Couldn't get sample rate for audio stream")
                as usize;

            self.channels = structure.get::<i32>("channels")
                .expect("Couldn't get sample channels for audio stream")
                as usize;

            self.drawingarea.queue_draw();
            self.media_ctl.show();
        }
        else {
            self.media_ctl.hide();
        }
    }
}
