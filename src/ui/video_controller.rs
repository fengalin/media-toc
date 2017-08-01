extern crate gtk;
use gtk::prelude::*;

extern crate cairo;

extern crate gstreamer as gst;
use gstreamer::*;

use std::ops::{Deref, DerefMut};

use ::media::Context;

use super::{MediaController, MediaHandler};


pub struct VideoController {
    media_ctl: MediaController,
    is_thumbnail_only: bool,
}


impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        VideoController {
            media_ctl: MediaController::new(
                builder.get_object("video-container").unwrap(),
                builder.get_object("video-drawingarea").unwrap()
            ),
            is_thumbnail_only: false,
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
        self.media_ctl.hide();
    }
}
