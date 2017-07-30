extern crate gtk;
use gtk::prelude::*;

extern crate cairo;

extern crate gstreamer as gst;
use gstreamer::*;

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::cell::RefCell;

use ::media::{Context, MediaHandler, VideoHandler};

use super::MediaController;


pub struct VideoController {
    media_ctl: MediaController,
    is_thumbnail_only: bool,
}


impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<VideoController>> {
        Rc::new(RefCell::new(VideoController {
            media_ctl: MediaController::new(
                builder.get_object("video-container").unwrap(),
                builder.get_object("video-drawingarea").unwrap()
            ),
            is_thumbnail_only: false,
        }))
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

impl VideoHandler for VideoController {
    fn new_video_stream(&mut self, context: &mut Context) {
        // this is how to get src_pad
        // let src = pipeline.clone().dynamic_cast::<Bin>().unwrap().get_by_name("src").unwrap();
        /*
        let queue = gst::ElementFactory::make("queue", None).unwrap();
        let convert = gst::ElementFactory::make("videoconvert", None).unwrap();
        let scale = gst::ElementFactory::make("videoscale", None).unwrap();
        let sink = gst::ElementFactory::make("autovideosink", None).unwrap();

        let elements = &[&queue, &convert, &scale, &sink];
        context.pipeline.add_many(elements).unwrap();
        gst::Element::link_many(elements).unwrap();

        for e in elements {
            e.sync_state_with_parent().unwrap();
        }

        let sink_pad = queue.get_static_pad("sink").unwrap();
        assert_eq!(src_pad.link(&sink_pad), gst::PadLinkReturn::Ok);*/

        self.media_ctl.show();
    }

    fn new_video_frame(&mut self, context: &Context) {
    }
}
