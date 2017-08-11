extern crate cairo;

extern crate glib;

extern crate gstreamer as gst;
use gstreamer::{BinExt, ElementExt, PadExt};

extern crate gtk;
use gtk::{BoxExt, ContainerExt, WidgetExt};

use ::media::Context;

pub struct VideoController {
    container: gtk::Container,
    video_box: gtk::Box,
}

impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        VideoController {
            container: builder.get_object("video-container").unwrap(),
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

    pub fn new_media(&mut self, ctx: &Context) {
        // TODO: test info in order to avoid checking pipeline directly
        if let Some(video_sink) = ctx.pipeline.get_by_name("video_sink") {
            println!("\nVideo sink caps:");
            for cap in video_sink.get_static_pad("sink").unwrap().get_current_caps().unwrap().iter() {
                println!("\t{:?}", cap);
            }
            self.container.show();
        }
        else {
            self.container.hide();
        }
    }
}
