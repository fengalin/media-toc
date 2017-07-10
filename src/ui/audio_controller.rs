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

pub struct AudioController {
    media_ctl: MediaController,
    drawingarea: gtk::DrawingArea,
    message: String,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<AudioController>> {
        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let ac = Rc::new(RefCell::new(AudioController {
            media_ctl: MediaController::new(builder.get_object("audio-container").unwrap()),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
            message: "audio place holder".to_owned(),
        }));

        let ac_for_cb = ac.clone();
        ac.borrow().drawingarea.connect_draw(move |_, cairo_ctx| {
            ac_for_cb.borrow().draw(&cairo_ctx);
            Inhibit(false)
        });

        ac
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


impl Deref for AudioController {
	type Target = MediaController;

	fn deref(&self) -> &Self::Target {
		&self.media_ctl
	}
}

impl DerefMut for AudioController {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.media_ctl
	}
}

impl NotifiableMedia for AudioController {
    fn new_media(&mut self, context: &mut Context) {
        self.message = match context.audio_stream.as_mut() {
            Some(stream) => {
                self.show();
                println!("** Audio stream\n{:?}", &stream);
                format!("audio stream {}", stream.index)
            },
            None => {
                self.hide();
                "no audio stream".to_owned()
            },
        };

        self.drawingarea.queue_draw();
    }
}
