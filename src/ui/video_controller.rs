extern crate gtk;
use gtk::WidgetExt;

use std::rc::Rc;
use std::cell::RefCell;

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

    pub fn new_media(&mut self, context_rc: &Rc<RefCell<Context>>) {
        let context = context_rc.borrow();

        let has_video = context.info.lock()
                .expect("Failed to lock media info while initializing video controller")
                .video_best
                .is_some();

        if has_video {
            self.video_box.show_all();
            self.container.show();
        }
        else {
            self.container.hide();
        }
    }
}
