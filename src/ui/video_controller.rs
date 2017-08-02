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
        // TODO: show or hide depending on the presence of a video stream in the context
        self.media_ctl.hide();

        /*if let Some(video_sink) = new_ctx.pipeline.get_by_name("video_sink") {
            video_sink.set_property("widget", &glib::Value::from(video_area));
        }*/

        // else: no video sink for media
    }
}
