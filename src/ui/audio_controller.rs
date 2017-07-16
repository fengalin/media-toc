extern crate gtk;
extern crate cairo;

extern crate ffmpeg;

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;
use cairo::enums::{FontSlant, FontWeight};

use ::media::Context;
use ::media::AudioNotifiable;

use super::MediaNotifiable;
use super::MediaController;

pub struct AudioController {
    media_ctl: MediaController,
    drawingarea: gtk::DrawingArea,
    message: String,
    graph: Option<ffmpeg::filter::Graph>,
    frame: Option<ffmpeg::frame::Audio>,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<AudioController>> {
        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let ac = Rc::new(RefCell::new(AudioController {
            media_ctl: MediaController::new(builder.get_object("audio-container").unwrap()),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
            message: "audio place holder".to_owned(),
            graph: None,
            frame: None,
        }));

        let ac_for_cb = ac.clone();
        ac.borrow().drawingarea.connect_draw(move |_, cairo_ctx| {
            ac_for_cb.borrow().draw(&cairo_ctx);
            Inhibit(false)
        });

        ac
    }

    fn build_graph(&mut self, decoder: &ffmpeg::codec::decoder::Audio) {
        let mut graph = ffmpeg::filter::Graph::new();

        let args = format!("time_base={}:sample_rate={}:sample_fmt={}:channel_layout=0x{:x}",
            decoder.time_base(), decoder.rate(), decoder.format().name(), decoder.channel_layout().bits());

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
            out_pad.set_sample_format(ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed));
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

    fn convert_to_pcm16(&mut self, frame: ffmpeg::frame::Audio) -> Result<ffmpeg::frame::Audio, String> {
        let mut graph = self.graph.as_mut().unwrap();
        graph.get("in").unwrap().source().add(&frame).unwrap();

        let mut frame_pcm = ffmpeg::frame::Audio::empty();
        while let Ok(..) = graph.get("out").unwrap().sink().frame(&mut frame_pcm) {
        }

        assert!(frame_pcm.planes() == 1); // samples converted to I16::Packed
        {
            let channels_nb = frame_pcm.channels() as usize;
            let mut channels = Vec::with_capacity(channels_nb);
            for index in 0..channels_nb {
                // TODO: reserve target capacity
                channels.push(Vec::new());
            }
            // FIXME: this doesn't seem rust like iteration to me
            let mut keep_going = true;
            let mut sample_iter = frame_pcm.data(0).iter();
            while keep_going {
                for index in 0..channels_nb {
                    let mut sample: i16 = 0;
                    if let Some(sample_byte) = sample_iter.next() {
                        sample = *sample_byte as i16;
                    }
                    else {
                        keep_going = false;
                        break;
                    }

                    if let Some(sample_byte) = sample_iter.next() {
                        // TODO: validate this
                        channels[index].push(sample + ((*sample_byte as i16) << 8));
                    }
                    else {
                        keep_going = false;
                        break;
                    }
                }
            }

            for index in 0..channels_nb {
                println!("\tChannel {}", index);
                let mut sample_str = String::new();
                for sample in &channels[index] {
                    sample_str += &format!("{:4x} ", sample);
                }
                println!("\t\tsamples {}", sample_str);
            }
        }

        Ok(frame_pcm)
    }

    fn draw(&self, cr: &cairo::Context) {
        let allocation = self.drawingarea.get_allocation();
        cr.scale(allocation.width as f64, allocation.height as f64);

        cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
        cr.set_font_size(0.07);

        cr.move_to(0.1, 0.53);
        cr.show_text(&self.message);
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
        self.graph = None;
        self.frame = None;

        match context.audio_decoder {
            Some((stream_index, ref decoder)) => {
                self.build_graph(decoder);
                self.show();
            },
            None => {
                self.hide();
            }
        };

        self.drawingarea.queue_draw();
    }
}

impl AudioNotifiable for AudioController {
    fn new_audio_frame(&mut self, frame: ffmpeg::frame::Audio) {
        match self.convert_to_pcm16(frame) {
            Ok(frame_pcm) => self.frame = Some(frame_pcm),
            Err(error) =>  panic!("\tError converting to pcm: {:?}", error),
        }
    }
}
