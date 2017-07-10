extern crate gtk;
extern crate cairo;

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;
use cairo::enums::{FontSlant, FontWeight};

use ::media::Context;

use super::NotifiableMedia;
use super::MediaController;

pub struct VideoController {
    media_ctl: MediaController,
    drawingarea: gtk::DrawingArea,
    message: String,
}


impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<VideoController>> {
        // need a RefCell because the callbacks will use immutable versions of vc
        // when the UI controllers will get a mutable version from time to time
        let vc = Rc::new(RefCell::new(VideoController {
            media_ctl: MediaController::new(builder.get_object("video-container").unwrap()),
            drawingarea: builder.get_object("video-drawingarea").unwrap(),
            message: "video place holder".to_owned(),
        }));

        let vc_for_cb = vc.clone();
        vc.borrow().drawingarea.connect_draw(move |_, cairo_ctx| {
            vc_for_cb.borrow().draw(&cairo_ctx);
            Inhibit(false)
        });

        vc
    }

    fn draw(&self, cr: &cairo::Context) {
        let allocation = self.drawingarea.get_allocation();
        cr.scale(allocation.width as f64, allocation.height as f64);

        cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
        cr.set_font_size(0.07);

        cr.move_to(0.1, 0.53);
        cr.show_text(&self.message);
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

impl NotifiableMedia for VideoController {
    fn notify_new_media(&mut self, context: &mut Context) {
        self.message = match context.video_stream.as_mut() {
            Some(stream) => {
                self.show();
                println!("** Video stream\n{:?}", &stream);
                format!("video stream {}", stream.index)
            },
            None => {
                self.hide();
                "no video stream".to_owned()
            },
        };

        self.drawingarea.queue_draw();
    }
}
