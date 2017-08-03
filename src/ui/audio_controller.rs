extern crate gtk;

extern crate cairo;

extern crate gstreamer as gst;
use gstreamer::BinExt;

use std::ops::{Deref, DerefMut};

use ::media::Context;

use super::{MediaController, MediaHandler};

pub struct AudioController {
    media_ctl: MediaController,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Self {
        AudioController {
            media_ctl: MediaController::new(
                builder.get_object("audio-container").unwrap(),
                builder.get_object("audio-drawingarea").unwrap()
            ),
        }
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
    fn new_media(&mut self, context: &Context) {
        if let Some(audio_sink) = context.pipeline.get_by_name("audio_sink") {
            self.media_ctl.show();
        }
        else {
            self.media_ctl.hide();
        }
    }
}
