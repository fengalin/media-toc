extern crate cairo;

extern crate glib;

extern crate gstreamer as gst;
use gstreamer::{BinExt, ElementExt, PadExt};

extern crate gtk;
use gtk::{BoxExt, ContainerExt, WidgetExt};

use std::ops::{Deref, DerefMut};

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

    pub fn have_widget(&self, widget_val: glib::Value) {
        for child in self.video_box.get_children() {
            self.video_box.remove(&child);
        }

        let widget = widget_val.get::<gtk::Widget>()
            .expect("Failed to get GstGtkWidget glib::Value as gtk::Widget");
        self.video_box.pack_start(&widget, true, true, 0);
        self.video_box.show_all();
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
    fn new_media(&mut self, ctx: &Context) {
        // TODO: test info in order to avoid checking pipeline directly
        if let Some(video_sink) = ctx.pipeline.get_by_name("video_sink") {
            println!("\nVideo sink caps:");
            for cap in video_sink.get_static_pad("sink").unwrap().get_current_caps().unwrap().iter() {
                println!("\t{:?}", cap);
            }
            self.media_ctl.show();
        }
        else {
            self.media_ctl.hide();
        }
    }
}
