extern crate gtk;
use gtk::prelude::*;

extern crate cairo;

extern crate gstreamer as gst;
use gstreamer::*;

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::cell::RefCell;

use ::media::{Context, MediaHandler, AudioHandler};

use super::MediaController;

pub struct AudioController {
    media_ctl: MediaController,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<AudioController>> {
        Rc::new(RefCell::new(AudioController {
            media_ctl: MediaController::new(
                builder.get_object("audio-container").unwrap(),
                builder.get_object("audio-drawingarea").unwrap()
            ),
        }))
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
        self.media_ctl.hide();
        // TODO: set an Option to indicate that no stream is initialized
    }
}

impl AudioHandler for AudioController {
    fn new_audio_stream(&mut self, context: &mut Context) {
        // TODO: check an Option to verify if a stream is already initialized

        // this is how to get src_pad
        // let src = pipeline.clone().dynamic_cast::<Bin>().unwrap().get_by_name("src").unwrap();
        /*
        // TODO: we will need 2 pipeline branches (there must be another
        // name for this) for audio:
        // 1- will go to the AudioController draw method
        // 2- will go to the audiosink
        // TODO: find out how to name the audio sink so that the name
        // of the application appears in DE audio mix application
        let queue = gst::ElementFactory::make("queue", None).unwrap();
        let convert = gst::ElementFactory::make("audioconvert", None).unwrap();
        let resample = gst::ElementFactory::make("audioresample", None).unwrap();
        let sink = gst::ElementFactory::make("autoaudiosink", None).unwrap();

        let elements = &[&queue, &convert, &resample, &sink];
        context.pipeline.add_many(elements).unwrap();
        gst::Element::link_many(elements).unwrap();

        for e in elements {
            e.sync_state_with_parent().unwrap();
        }

        let sink_pad = queue.get_static_pad("sink").unwrap();
        assert_eq!(src_pad.link(&sink_pad), gst::PadLinkReturn::Ok);*/

        self.media_ctl.show();
    }

    fn new_audio_frame(&mut self, context: &Context) {
    }
}
