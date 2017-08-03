extern crate gtk;

extern crate cairo;

extern crate gstreamer as gst;
use gstreamer::BinExt;

use std::ops::{Deref, DerefMut};

use ::media::Context;

use super::{MediaController, MediaHandler};


pub struct VideoController {
    media_ctl: MediaController,
}


impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        VideoController {
            media_ctl: MediaController::new(
                builder.get_object("video-container").unwrap(),
                builder.get_object("video-drawingarea").unwrap()
            ),
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
        if let Some(video_sink) = context.pipeline.get_by_name("video_sink") {
            // TODO: replace "video_sink" with something like this:
            /*let sink = if let Some(gtkglsink) = ElementFactory::make("gtkglsink", None) {
                let glsinkbin = ElementFactory::make("glsinkbin", "video_sink").unwrap();
                glsinkbin
                    .set_property("sink", &gtkglsink.to_value())
                    .unwrap();
                glsinkbin
            } else {
                let sink = ElementFactory::make("gtksink", "video_sink").unwrap();
                sink
            };*/
            self.media_ctl.show();
        }
        else {
            self.media_ctl.hide();
        }
    }
}
