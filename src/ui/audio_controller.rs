extern crate glib;

extern crate gtk;
use gtk::{Inhibit, WidgetExt};

extern crate cairo;

use std::rc::{Rc, Weak};
use std::cell::RefCell;

use std::collections::vec_deque::{VecDeque};

use std::ops::{Deref, DerefMut};

use ::media::{Context, AudioBuffer};

use super::{MediaController, MediaHandler};

pub struct AudioController {
    media_ctl: MediaController,
    drawingarea: gtk::DrawingArea,

    circ_buffer: VecDeque<AudioBuffer>,
    sample_offset: f64,
    samples_nb: f64,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let ac = Rc::new(RefCell::new(AudioController {
            media_ctl: MediaController::new(
                builder.get_object("audio-container").unwrap(),
            ),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            circ_buffer: VecDeque::new(),
            sample_offset: 0f64,
            samples_nb: 0f64,
        }));

        {
            let ac_ref = ac.borrow();
            let ac_weak = Rc::downgrade(&ac);
            ac_ref.drawingarea.connect_draw(move |ref drawing_area, ref cairo_ctx| {
                let ac = ac_weak.upgrade()
                    .expect("Main controller is no longer available for select_media");
                return ac.borrow().draw(drawing_area, cairo_ctx);
            });
        }

        ac
    }

    pub fn clear(&mut self) {
        self.circ_buffer.clear();
        self.sample_offset = 0f64;
        self.samples_nb = 0f64;
    }

    pub fn have_buffer(&mut self, buffer: AudioBuffer) {
        // Firt approximation: suppose the buffers come in ordered
        if self.sample_offset == 0f64 {
            self.sample_offset = buffer.sample_offset as f64;
        }
        self.samples_nb += buffer.samples_nb as f64;

        self.circ_buffer.push_back(buffer);
    }

    fn draw(&self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) -> Inhibit {
        if self.circ_buffer.len() == 0 {
            return Inhibit(false);
        }

        let sample_nb = self.samples_nb.min(5_000f64);
        let sample_dyn = 1024f64;

        let allocation = drawing_area.get_allocation();
        cr.scale(
            allocation.width as f64 / sample_nb,
            allocation.height as f64 / 2f64 / sample_dyn,
        );
        cr.set_line_width(1f64);

        for ref buffer in self.circ_buffer.iter() {
            for ref channel in buffer.channels.iter() {
                let colors = vec![(0.8f64, 0.8f64, 0.8f64), (0.8f64, 0f64, 0f64)][channel.id];
                cr.set_source_rgb(colors.0, colors.1, colors.2);

                let mut x = buffer.sample_offset as f64 - self.sample_offset;
                let mut is_first = true;
                for &sample in channel.iter() {
                    let y = sample_dyn * (1f64 - sample);
                    if !is_first {
                        cr.line_to(x, y);
                    } else {
                        cr.move_to(x, y);
                        is_first = false;
                    }
                    x += 1f64;
                    if x > sample_nb { break; }
                }

                cr.stroke();
            }
        }

        Inhibit(false)
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

impl MediaHandler for AudioController {
    fn new_media(&mut self, ctx: &Context) {
        if let Some(audio_buffer) = self.circ_buffer.get(0) {
            println!("\n{:?}", audio_buffer.caps);
            self.drawingarea.queue_draw();
            self.media_ctl.show();
        }
        else {
            self.media_ctl.hide();
        }
    }
}
