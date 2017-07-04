extern crate gtk;
extern crate cairo;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use cairo::enums::{FontSlant, FontWeight};

pub struct AudioController {
    area: gtk::DrawingArea,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<AudioController>> {
        let ac = AudioController {
            area: builder.get_object("audio-drawingarea").expect("Couldn't find audio-drawingarea"),
        };

        // connect draw event - tricky because of the callback
        let area = ac.area.clone(); // clone needed because ac will be moved in next statement
        let ac_rc = Rc::new(RefCell::new(ac));
        let cloned_ac_rc = ac_rc.clone(); // clone needed otherwise ac_rc would be moved in closure
        area.connect_draw(move |drawing_area, cairo_ctx| {
            let ac = cloned_ac_rc.borrow(); // TODO: or is it ac_rc?
            ac.draw(&cairo_ctx);
            Inhibit(false)
        });

        ac_rc
    }

    fn draw(&self, cr: &cairo::Context) {
        let allocation = self.area.get_allocation();
        cr.scale(allocation.width as f64, allocation.height as f64);

        cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
        cr.set_font_size(0.07);

        cr.move_to(0.1, 0.53);
        cr.show_text("audio place holder");
    }
}


