extern crate cairo;

extern crate glib;

extern crate gtk;
use gtk::{ContainerExt, WidgetExt};

use ::media::Context;

pub struct VideoController {
    container: gtk::Container,
    pub video_box: gtk::Box,
}

impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        VideoController {
            container: builder.get_object("video-container").unwrap(),
            video_box: builder.get_object("video-box").unwrap(),
        }
    }

    pub fn cleanup(&self) {
        for child in self.video_box.get_children() {
            self.video_box.remove(&child);
        }
    }

    pub fn new_media(&mut self, ctx: &Context) {
        let has_video = {
            let ctx_info = &ctx.info.lock()
                .expect("Failed to lock media info while initializing video controller");
            ctx_info.video_best.is_some()
        };

        if has_video {
            self.video_box.show_all();
            self.container.show();
        }
        else {
            self.container.hide();
        }
    }
}
