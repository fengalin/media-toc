extern crate glib;

extern crate gtk;
use gtk::{Inhibit, WidgetExt};

extern crate cairo;

use std::iter::FromIterator;

use std::rc::Rc;
use std::cell::RefCell;

use std::collections::vec_deque::{VecDeque};

use ::media::{Context, AudioBuffer};

pub struct AudioController {
    container: gtk::Container,
    drawingarea: gtk::DrawingArea,

    sample_buffer: VecDeque<f64>,
    buffer_duration: f64,
    offset: f64,
    relative_pos: f64,
    samples_nb: usize,
    sample_pixel_step: f64,
    iter_since_adjust: usize,
    max_iter_before_adjust: usize,

    channels: usize,
    sample_duration: f64,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            sample_buffer: VecDeque::new(),
            buffer_duration: 0f64,
            relative_pos: 0f64,
            samples_nb: 0,
            sample_pixel_step: 0f64,
            iter_since_adjust: 0,
            max_iter_before_adjust: 5,

            channels: 0,
            offset: 0f64,
            sample_duration: 0f64,
        }));

        {
            let this_ref = this.borrow();
            let this_weak = Rc::downgrade(&this);
            this_ref.drawingarea.connect_draw(move |drawing_area, cairo_ctx| {
                let this = this_weak.upgrade()
                    .expect("Main controller is no longer available for select_media");
                let result = this.borrow_mut().draw(drawing_area, cairo_ctx);
                result
            });
        }

        this
    }

    pub fn clear(&mut self) {
        self.sample_buffer.clear();
    }

    pub fn new_media(&mut self, ctx: &Context) {
        if !self.sample_buffer.is_empty() {
            println!("\nAudio stream: channels {}", self.channels);
            self.drawingarea.queue_draw();
            self.container.show();
        }
        else {
            self.container.hide();
        }
    }

    pub fn have_buffer(&mut self, mut buffer: AudioBuffer) {
        // First approximation: suppose the buffers come in ordered
        if self.sample_buffer.is_empty() {
            self.buffer_duration = 0f64;
            self.offset = buffer.pts;
            self.relative_pos = 0f64;
            self.samples_nb = 0;
            self.sample_pixel_step = 1f64;
            self.iter_since_adjust = 0;

            self.channels = buffer.caps.channels;
            self.sample_duration = buffer.caps.sample_duration / 1_000_000_000f64;
        }

        self.samples_nb += buffer.samples_nb;
        self.buffer_duration = self.samples_nb as f64 * self.sample_duration;

        let mut samples_vecdeque = VecDeque::from_iter(buffer.samples.drain(..));
        self.sample_buffer.append(&mut samples_vecdeque);

        // TODO: this depends on the actual position in the stream
        /*while self.samples_nb > self.samples_max {
            let prev_buffer = self.sample_buffer.pop_front()
                .expect("Unconsistent samples nb in audio circular buffer");
            self.samples_nb -= prev_buffer.samples_nb as f64;
            match self.sample_buffer.front() {
                Some(first_buffer) => self.sample_offset = first_buffer.sample_offset as f64,
                None => (),
            };
        }*/
    }

    pub fn have_position(&mut self, position: f64) {
        let relative_pos = (position - self.offset) / 1_000_000_000f64;
        if self.relative_pos != relative_pos {
            self.relative_pos = relative_pos;
            self.drawingarea.queue_draw();
        }
    }

    fn draw(&mut self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) -> Inhibit {
        if self.sample_buffer.is_empty() {
            return Inhibit(false);
        }

        let display_window_duration = 2f64; // 2s TODO: adapt according to zoom
        let diplay_window_sample_nb_f = display_window_duration / self.sample_duration;
        let display_duration = self.buffer_duration.min(display_window_duration);
        let diplay_sample_nb = (display_duration / self.sample_duration) as usize;

        let allocation = drawing_area.get_allocation();
        let width_f = allocation.width as f64;
        cr.scale(
            width_f / display_window_duration,
            allocation.height as f64 / 2f64,
        );
        cr.set_line_width(0.002f64);

        let half_duration = display_duration / 2f64;
        // Take this opportunity to adjust sample pixel step in order
        // to accomodate to computation requirements
        self.iter_since_adjust += 1;
        let first_display_pos = if self.relative_pos > self.buffer_duration - half_duration {
            // TODO: don't adjust sample_pixel when reaching the end of file (need total duration)
            if self.iter_since_adjust > self.max_iter_before_adjust
                && self.relative_pos > self.buffer_duration - 0.25f64 * display_duration
            {
                self.sample_pixel_step += 1f64;
                self.iter_since_adjust = 0;
            }
            self.buffer_duration - display_duration
        } else if self.relative_pos > half_duration {
            if self.iter_since_adjust > self.max_iter_before_adjust
                && self.sample_pixel_step > 1f64 && self.relative_pos > self.buffer_duration - 0.25f64 * display_duration
            {
                self.sample_pixel_step -= 1f64;
                self.iter_since_adjust = 0;
            }
            self.relative_pos - half_duration
        } else {
            0f64
        };

        let sample_step_f = if diplay_window_sample_nb_f > width_f / self.sample_pixel_step {
            (diplay_window_sample_nb_f / width_f * self.sample_pixel_step).trunc()
        } else {
            1f64
        };
        let sample_step = sample_step_f as usize;
        // Define the first sample as a multiple of sample_step
        // In order to avoid flickering when origin changes between redraws
        let first_sample = (first_display_pos / self.sample_duration / sample_step_f).trunc() as usize * sample_step;
        let first_idx = first_sample * self.channels;
        let last_idx = first_idx + diplay_sample_nb * self.channels;

        let duration_step = sample_step_f * self.sample_duration;
        let idx_step = sample_step * self.channels;


        let colors = vec![(0.9f64, 0.9f64, 0.9f64), (0.9f64, 0f64, 0f64)];
        for channel in 0..self.channels {
            let color = if channel < 2 {
                colors[channel]
            } else {
                (0f64, 0f64, 0.2f64 * channel as f64)
            };
            cr.set_source_rgb(color.0, color.1, color.2);

            let mut sample_relative_ts = 0f64;
            let mut sample_idx = first_idx + channel;
            while sample_idx < last_idx {
                let y = self.sample_buffer[sample_idx];
                if sample_idx > 0 {
                    cr.line_to(sample_relative_ts, y);
                } else {
                    cr.move_to(sample_relative_ts, y);
                }

                sample_relative_ts += duration_step;
                sample_idx += idx_step;
            }

            cr.stroke();
        }

        // draw current pos
        let x = self.relative_pos - first_display_pos;
        cr.set_source_rgb(1f64, 1f64, 0f64);
        cr.set_line_width(0.004f64);
        cr.move_to(x, 0f64);
        cr.line_to(x, 2f64);
        cr.stroke();

        Inhibit(false)
    }
}
