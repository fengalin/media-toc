extern crate cairo;

extern crate gstreamer as gst;
use gstreamer::BinExt;

extern crate gtk;

use std::ops::{Deref, DerefMut};

use std::sync::mpsc::Sender;

use ::media::Context;

use super::{MediaController, MediaHandler};


pub struct VideoController {
    media_ctl: MediaController,
    pub video_box: gtk::Box,
}

impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        VideoController {
            media_ctl: MediaController::new(
                builder.get_object("video-container").unwrap(),
            ),
            video_box: builder.get_object("video-box").unwrap(),
        }
    }
}

impl Deref for VideoController {
	type Target = MediaController;

	fn deref(&self) -> &Self::Target {
		&self.media_ctl
	}
}

impl DerefMut for VideoController {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.media_ctl
	}
}

impl MediaHandler for VideoController {
    fn new_media(&mut self, context: &Context) {
        if let Some(_) = context.pipeline.get_by_name("video_sink") {
            self.media_ctl.show();
        }
        else {
            self.media_ctl.hide();
        }
    }
}
