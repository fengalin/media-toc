extern crate gtk;
extern crate cairo;
extern crate ffmpeg;

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;
use cairo::enums::{FontSlant, FontWeight};

pub struct VideoController {
    area: gtk::DrawingArea,
    message: String,
}


impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<VideoController>> {
        ffmpeg::init().expect("Couln't initialize FFMPEG");

        // need RefCell on because the callbacks will use immutable versions of vc
        // when the UI controllers will get a mutable version from time to time
        let vc = Rc::new(RefCell::new(VideoController {
            area: builder.get_object("video-drawingarea").expect("Couldn't find video-drawingarea"),
            message: String::from("video place holder"),
        }));

        let vc_for_cb = vc.clone();
        vc.borrow().area.connect_draw(move |_, cairo_ctx| {
            let vc = vc_for_cb.borrow();
            vc.draw(&cairo_ctx);
            Inhibit(false)
        });

        vc
    }

    fn draw(&self, cr: &cairo::Context) {
        let allocation = self.area.get_allocation();
        cr.scale(allocation.width as f64, allocation.height as f64);

        cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
        cr.set_font_size(0.07);

        cr.move_to(0.1, 0.53);
        cr.show_text(&self.message);
    }

    pub fn notify_new_media(&mut self) {
        self.message = String::from("new media opened");
        self.area.queue_draw();
    }
}


