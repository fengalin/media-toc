extern crate gtk;

extern crate cairo;

extern crate gstreamer as gst;
use gstreamer::{BinExt, ElementExt, PadExt};

use std::collections::vec_deque::{VecDeque};

use std::ops::{Deref, DerefMut};

use ::media::Context;

use super::{MediaController, MediaHandler};

pub struct AudioController {
    media_ctl: MediaController,
    drawingarea: gtk::DrawingArea,

    circ_buffer: VecDeque<gst::Buffer>,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Self {
        AudioController {
            media_ctl: MediaController::new(
                builder.get_object("audio-container").unwrap(),
            ),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            circ_buffer: VecDeque::new(),
        }
    }

    pub fn have_caps(&self, caps: gst::Caps) {
        println!("Audio caps: {:?}", caps);
    }

    pub fn clear(&mut self) {
        self.circ_buffer.clear();
    }

    pub fn have_buffer(&mut self, buffer: gst::Buffer) {
        self.circ_buffer.push_back(buffer);
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
            println!("\nAudio sink caps:");
            for cap in audio_sink.get_static_pad("sink").unwrap().get_current_caps().unwrap().iter() {
                println!("\t{:?}", cap);
            }
            self.media_ctl.show();
        }
        else {
            self.media_ctl.hide();
        }
    }
}
