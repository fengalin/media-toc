extern crate gtk;

extern crate cairo;

extern crate gstreamer as gst;
use gstreamer::BinExt;

use std::ops::{Deref, DerefMut};

use ::media::{Context, MediaInfo};

use super::{MediaController, MediaHandler};

pub struct AudioController {
    media_ctl: MediaController,
    drawingarea: gtk::DrawingArea,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Self {
        AudioController {
            media_ctl: MediaController::new(
                builder.get_object("audio-container").unwrap(),
            ),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
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
    fn new_media(&mut self, context: &Context, info: &MediaInfo) {
        // TODO: test info in order to avoid checking pipeline directly
        if let Some(_) = context.pipeline.get_by_name("audio_sink") {
            self.media_ctl.show();
        }
        else {
            self.media_ctl.hide();
        }
    }
}
