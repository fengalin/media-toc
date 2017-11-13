extern crate gtk;
use gtk::{BoxExt, WidgetExt};

use std::rc::Rc;
use std::cell::RefCell;

use media::PlaybackContext;

use super::MainController;

pub struct VideoController {
    container: gtk::Box,
}

impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        let container: gtk::Box = builder.get_object("video-container").unwrap();
        let video_widget = PlaybackContext::get_video_widget();
        container.pack_start(&video_widget, true, true, 0);
        container.reorder_child(&video_widget, 0);

        VideoController { container: container }
    }

    pub fn register_callbacks(&self, _: &Rc<RefCell<MainController>>) {}

    pub fn new_media(&mut self, context: &PlaybackContext) {
        let has_video = context
            .info
            .lock()
            .expect(
                "Failed to lock media info while initializing video controller",
            )
            .video_best
            .is_some();

        if has_video {
            self.container.show();
        } else {
            self.container.hide();
        }
    }
}
