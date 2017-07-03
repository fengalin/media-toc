extern crate gtk;
extern crate cairo;

use gtk::prelude::*;
use cairo::enums::{FontSlant, FontWeight};

pub struct AudioController {
    da: gtk::DrawingArea,
}

// TODO: figure out how to make this callback a part of the impl
fn draw_something(da: &gtk::DrawingArea, cr: &cairo::Context) -> Inhibit {
    let allocation = da.get_allocation();
    cr.scale(allocation.width as f64, allocation.height as f64);

    cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
    cr.set_font_size(0.07);

    cr.move_to(0.1, 0.53);
    cr.show_text("audio place holder");

    Inhibit(false)
}


impl AudioController {
    pub fn new(builder: &gtk::Builder) -> AudioController {
        let result = AudioController {
            da: builder.get_object("audio-drawingarea").unwrap(),
        };
        result.da.connect_draw(draw_something);

        result
    }
}


