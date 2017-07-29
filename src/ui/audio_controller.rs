extern crate gtk;
extern crate cairo;

extern crate gstreamer;

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;

use ::media::Context;
use ::media::AudioNotifiable;

use super::MediaNotifiable;
use super::MediaController;

pub struct AudioController {
    media_ctl: MediaController,
    drawingarea: gtk::DrawingArea,
     /*graph: Option<ffmpeg::filter::Graph>,
    frame: Option<ffmpeg::frame::Audio>, */
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<AudioController>> {
        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let ac = Rc::new(RefCell::new(AudioController {
            media_ctl: MediaController::new(builder.get_object("audio-container").unwrap()),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
            /*graph: None,
            frame: None,*/
        }));

        let ac_for_cb = ac.clone();
        ac.borrow().drawingarea.connect_draw(move |ref drawing_area, ref cairo_ctx| {
            ac_for_cb.borrow().draw(drawing_area, cairo_ctx);
            Inhibit(false)
        });

        ac
    }

    /*
    fn build_graph(&mut self, decoder: &ffmpeg::codec::decoder::Audio) {
        let mut graph = ffmpeg::filter::Graph::new();

        // Fix broken channel layouts
        let channel_layout = match decoder.channel_layout().bits() {
            0 => match decoder.channels() {
                1 => ffmpeg::channel_layout::MONO,
                2 => ffmpeg::channel_layout::FRONT_LEFT | ffmpeg::channel_layout::FRONT_RIGHT | ffmpeg::channel_layout::STEREO,
                _ => panic!("Unknown channel layout"),
            },
            _ => decoder.channel_layout(),
        };
        let args = format!("time_base={}:sample_rate={}:sample_fmt={}:channel_layout=0x{:x}",
            decoder.time_base(), decoder.rate(), decoder.format().name(), channel_layout.bits());

        let in_filter = ffmpeg::filter::find("abuffer").unwrap();
        match graph.add(&in_filter, "in", &args) {
            Ok(_) => (),
            Err(error) => panic!("Error adding in pad: {:?}", error),
        }

        let out_filter = ffmpeg::filter::find("abuffersink").unwrap();
        match graph.add(&out_filter, "out", "") {
            Ok(_) => (),
            Err(error) => panic!("Error adding out pad: {:?}", error),
        }
        {
            let mut out_pad = graph.get("out").unwrap();
            out_pad.set_sample_format(ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Planar));
        }

        {
            let in_parser;
            match graph.output("in", 0) {
                Ok(value) => in_parser = value,
                Err(error) => panic!("Error getting output for in pad: {:?}", error),
            }
            let out_parser;
            match in_parser.input("out", 0) {
                Ok(value) => out_parser = value,
                Err(error) => panic!("Error getting input for out pad: {:?}", error),
            }
            match out_parser.parse("anull") {
                Ok(_) => (),
                Err(error) => panic!("Error parsing format: {:?}", error),
            }
        }

        match graph.validate() {
            Ok(_) => self.graph = Some(graph),
            Err(error) => panic!("Error validating graph: {:?}", error),
        }
    }
    */

    /*
    fn convert_to_pcm16(&mut self, frame: &ffmpeg::frame::Audio) -> Result<ffmpeg::frame::Audio, String> {
        let mut graph = self.graph.as_mut().unwrap();
        graph.get("in").unwrap().source().add(&frame).unwrap();

        let mut frame_pcm = ffmpeg::frame::Audio::empty();
        while let Ok(..) = graph.get("out").unwrap().sink().frame(&mut frame_pcm) {
        }

        match frame_pcm.pts() {
            Some(pts) => println!("\t\tpts {}", pts),
            None => (),
        }
        match frame_pcm.timestamp() {
            Some(timestamp) => println!("\t\ttimestamp {}", timestamp),
            None => (),
        }

        Ok(frame_pcm)
    }
    */

    fn draw(&self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) {
        /*
        match self.frame {
            Some(ref frame) => {
                let allocation = drawing_area.get_allocation();
                let offset = (::std::i16::MAX / 2) as i32;
                cr.scale(
                    allocation.width as f64 / frame.samples() as f64,
                    allocation.height as f64 / 2f64 / offset as f64,
                );
                cr.set_line_width(1f64);

                let mut ymin = offset as f64;
                let mut ymax = 0f64;
                let planes_nb = frame.planes();
                for index in 0..planes_nb {
                    let colors = vec![(0.8f64, 0.8f64, 0.8f64), (0.8f64, 0f64, 0f64)][index];
                    cr.set_source_rgb(colors.0, colors.1, colors.2);

                    let mut is_first = true;
                    let mut x = 0f64;
                    for sample in frame.plane::<i16>(index) {
                        let y = (offset - *sample as i32) as f64;
                        match is_first {
                            true => {
                                cr.move_to(x, y);
                                is_first = false;
                            },
                            false => {
                                cr.line_to(x, y);
                            },
                        }
                        x += 1f64;

                        if y < ymin {
                            ymin = y;
                        }
                        if y > ymax {
                            ymax = y;
                        }
                    }
                    cr.stroke();
                }
            },
            None => (),
        } */
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

impl MediaNotifiable for AudioController {
    fn new_media(&mut self, context: &Context) {
        /*
        self.graph = None;
        self.frame = None;

        match context.audio_decoder.as_ref() {
            Some(decoder) => {
                self.build_graph(decoder);
                self.show();
            },
            None => {
                self.hide();
            }
        };
        */

        self.drawingarea.queue_draw();
    }
}

impl AudioNotifiable for AudioController {
    /*
    fn new_audio_frame(&mut self, frame: &ffmpeg::frame::Audio) {
        match self.convert_to_pcm16(frame) {
            Ok(frame_pcm) => self.frame = Some(frame_pcm),
            Err(error) =>  panic!("\tError converting to pcm: {:?}", error),
        }
    }
    */
}
