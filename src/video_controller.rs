extern crate gtk;
extern crate cairo;
extern crate ffmpeg;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use cairo::enums::{FontSlant, FontWeight};

pub struct VideoController {
    area: gtk::DrawingArea,
}


impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<VideoController>> {
        ffmpeg::init().expect("Couln't initialize FFMPEG");

        let vc = VideoController {
            area: builder.get_object("video-drawingarea").expect("Couldn't find video-drawingarea"),
        };

        // connect draw event - tricky because of the callback
        let area = vc.area.clone(); // clone needed because vc will be moved in next statement
        let vc_rc = Rc::new(RefCell::new(vc));
        let cloned_vc_rc = vc_rc.clone(); // clone needed otherwise vc_rc would be moved in closure
        area.connect_draw(move |drawing_area, cairo_ctx| {
            let vc = cloned_vc_rc.borrow(); // TODO: or is it vc_rc?
            vc.draw(&cairo_ctx);
            Inhibit(false)
        });

        vc_rc
    }

    fn draw(&self, cr: &cairo::Context) {
        let allocation = self.area.get_allocation();
        cr.scale(allocation.width as f64, allocation.height as f64);

        cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
        cr.set_font_size(0.07);

        cr.move_to(0.1, 0.53);
        cr.show_text("video place holder");
    }

}


