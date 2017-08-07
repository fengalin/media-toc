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
    channels: usize,
    sample_offset: f64,
    samples_nb: f64,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(AudioController {
            media_ctl: MediaController::new(
                builder.get_object("audio-container").unwrap(),
            ),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            circ_buffer: VecDeque::new(),
            channels: 0,
            sample_offset: 0f64,
            samples_nb: 0f64,
        }));

        {
            let this_ref = this.borrow();
            let this_weak = Rc::downgrade(&this);
            this_ref.drawingarea.connect_draw(move |ref drawing_area, ref cairo_ctx| {
                let this = this_weak.upgrade()
                    .expect("Main controller is no longer available for select_media");
                return this.borrow().draw(drawing_area, cairo_ctx);
            });
        }

        this
    }

    pub fn clear(&mut self) {
        self.circ_buffer.clear();
        self.channels = 0;
        self.sample_offset = 0f64;
        self.samples_nb = 0f64;
    }

    pub fn have_buffer(&mut self, buffer: AudioBuffer) {
        // Firt approximation: suppose the buffers come in ordered
        if self.sample_offset == 0f64 {
            self.sample_offset = buffer.sample_offset as f64;
            self.channels = buffer.caps.channels;
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

        let mut colors = vec![(0.9f64, 0.9f64, 0.9f64), (0.9f64, 0f64, 0f64)];
        for channel in 2..self.channels {
            colors.push((0f64, 0f64, 0.2f64 * channel as f64));
        }

        for ref buffer in self.circ_buffer.iter() {
            for ref channel in buffer.channels.iter() {
                let color = colors[channel.id];
                cr.set_source_rgb(color.0, color.1, color.2);

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
