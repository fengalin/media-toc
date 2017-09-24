extern crate gtk;
use gtk::{Inhibit, WidgetExt};

use std::rc::Rc;
use std::cell::RefCell;

use media::Context;

use super::MainController;

pub struct VideoController {
    container: gtk::Container,
    pub video_box: gtk::Box,
    drawingarea: Option<gtk::DrawingArea>,
}

impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        VideoController {
            container: builder.get_object("video-container").unwrap(),
            video_box: builder.get_object("video-box").unwrap(),
            drawingarea: Some(builder.get_object("video-drawingarea").unwrap()),
        }
    }

    pub fn register_callbacks(&self, _: &Rc<RefCell<MainController>>) {
        // draw
        self.drawingarea.as_ref().unwrap().connect_draw(|_, cairo_ctx| {
            // draw black background
            cairo_ctx.set_source_rgb(0f64, 0f64, 0f64);
            cairo_ctx.paint();
            Inhibit(true)
        });
    }

    pub fn new_media(&mut self, context: &Context) {
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

        // from now on, drawingarea will be either hidden
        // or replaced with a video widget
        self.drawingarea = None;
    }
}
