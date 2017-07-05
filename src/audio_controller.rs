extern crate gtk;
extern crate cairo;

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;
use cairo::enums::{FontSlant, FontWeight};

pub struct AudioController {
    area: gtk::DrawingArea,
    message: String,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<AudioController>> {
        // need RefCell on because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let ac = Rc::new(RefCell::new(AudioController {
            area: builder.get_object("audio-drawingarea").expect("Couldn't find audio-drawingarea"),
            message: String::from("audio place holder"),
        }));

        let ac_for_cb = ac.clone();
        ac.borrow().area.connect_draw(move |_, cairo_ctx| {
            let ac = ac_for_cb.borrow();
            ac.draw(&cairo_ctx);
            Inhibit(false)
        });

        ac
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


