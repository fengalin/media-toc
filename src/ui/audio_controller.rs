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

    stream_duration: f64,
    channels: usize,
    sample_duration: f64,
    stream_initialized: bool,

    sample_buffer: VecDeque<f64>,
    offset: f64,
    relative_offset: f64,
    buffer_duration: f64,
    has_received_last_buffer: bool,

    relative_pos: f64,
    sample_skip_step: f64,
    sample_per_step_f: f64,
    iter_since_adjust: usize,
    min_iter_before_adjust: usize,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            stream_duration: 0f64,
            channels: 0,
            sample_duration: 0f64,
            stream_initialized: false,

            sample_buffer: VecDeque::with_capacity(6 * 48_000), // 6s @ 48kHz
            offset: 0f64,
            relative_offset: 0f64,
            buffer_duration: 0f64,
            has_received_last_buffer: false,

            relative_pos: 0f64,
            sample_skip_step: 0f64,
            sample_per_step_f: 0f64,
            iter_since_adjust: 0,
            min_iter_before_adjust: 5,
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
        self.stream_initialized = false;
    }

    pub fn new_media(&mut self, ctx: &Context) {
        if !self.sample_buffer.is_empty() {
            self.stream_duration = ctx.get_duration() as f64 / 1_000_000_000f64;
            self.stream_initialized = true;
            self.drawingarea.queue_draw();
            self.container.show();
        }
        else {
            self.container.hide();
        }
    }

    pub fn have_buffer(&mut self, mut buffer: AudioBuffer) {
        // suppose the buffers come in order
        if self.sample_buffer.is_empty() {
            self.offset = buffer.pts;
            self.relative_offset = 0f64;
            self.buffer_duration = 0f64;
            self.has_received_last_buffer = false;

            self.relative_pos = 0f64;
            self.sample_skip_step = 1f64;
            self.sample_per_step_f = 1f64;
            self.iter_since_adjust = 0;

            self.channels = buffer.caps.channels;
            self.sample_duration = buffer.caps.sample_duration / 1_000_000_000f64;
        }

        self.buffer_duration = self.sample_buffer.len() as f64 * self.sample_duration;
        self.has_received_last_buffer = self.stream_initialized
            && self.relative_offset + self.buffer_duration >= self.stream_duration;

        let mut samples_vecdeque = VecDeque::from_iter(buffer.samples.drain(..));
        self.sample_buffer.append(&mut samples_vecdeque);

        if self.buffer_duration > 5f64 && self.relative_pos > 2f64 { // buffer larger than 5s
            // remove 2s worse of samples
            // can't purge big chunks otherwise it impacts drawing
            // need to comply with current sample_per_step_f in order to
            // avoid discontinuities during waveform rendering
            let sample_nb_to_remove_f = (2f64 / self.sample_duration / self.sample_per_step_f).trunc() * self.sample_per_step_f;
            let sample_nb_to_remove = sample_nb_to_remove_f as usize;
            self.sample_buffer.drain(..sample_nb_to_remove);

            let duration_removed = sample_nb_to_remove_f * self.sample_duration;
            self.buffer_duration -= duration_removed;
            self.offset += duration_removed * 1_000_000_000f64;
            self.relative_offset += duration_removed;
            self.relative_pos -= duration_removed;
            //println!("buffer duration: {}", self.buffer_duration);
        }
    }

    pub fn have_position(&mut self, position: f64) {
        let relative_pos = (position - self.offset) / 1_000_000_000f64;
        if self.relative_pos != relative_pos {
            self.relative_pos = relative_pos;
            self.drawingarea.queue_draw();
        }
    }

    pub fn force_redraw(&mut self) {
        self.sample_skip_step = 1f64;
        self.iter_since_adjust = 0;
        self.drawingarea.queue_draw();
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
        cr.set_line_width(0.0015f64);

        let half_duration = display_duration / 2f64;
        // Take this opportunity to adjust sample skip step in order
        // to accomodate to computation requirements
        self.iter_since_adjust += 1;
        let first_display_pos = if !self.has_received_last_buffer && self.relative_pos > self.buffer_duration {
            self.sample_skip_step += 1f64;
            self.iter_since_adjust = 0;
            self.buffer_duration - display_duration
        } else if self.relative_pos > self.buffer_duration - half_duration {
            if !self.has_received_last_buffer && self.iter_since_adjust > self.min_iter_before_adjust
                && self.relative_pos > self.buffer_duration - 0.25f64 * display_duration
            {
                self.sample_skip_step += 1f64;
                self.iter_since_adjust = 0;
            }
            self.buffer_duration - display_duration
        } else if self.relative_pos > half_duration {
            // FIXME: need a real algorithm to stabilize
            /*if self.iter_since_adjust > self.min_iter_before_adjust
                && self.sample_skip_step > 1f64 && self.relative_pos < self.buffer_duration - 0.20f64 * display_duration
            {
                self.sample_skip_step -= 1f64;
                self.iter_since_adjust = 0;
                self.min_iter_before_adjust *= 2;
            }*/
            self.relative_pos - half_duration
        } else {
            0f64
        };

        self.sample_per_step_f = if diplay_window_sample_nb_f > width_f / self.sample_skip_step {
            (diplay_window_sample_nb_f / width_f * self.sample_skip_step).trunc()
        } else {
            1f64
        };
        let sample_step = self.sample_per_step_f as usize;
        // Define the first sample as a multiple of sample_step
        // In order to avoid flickering when origin changes between redraws
        let first_sample = (first_display_pos / self.sample_duration / self.sample_per_step_f).trunc() as usize * sample_step;
        let last_sample = first_sample + diplay_sample_nb;

        let duration_step = self.sample_per_step_f * self.sample_duration;

        cr.set_source_rgb(0.8f64, 0.8f64, 0.8f64);

        let mut sample_relative_ts = 0f64;
        let mut sample_idx = first_sample;

        cr.move_to(sample_relative_ts, self.sample_buffer[sample_idx]);
        sample_relative_ts += duration_step;
        sample_idx += sample_step;

        while sample_idx < last_sample {
            cr.line_to(sample_relative_ts, self.sample_buffer[sample_idx]);
            sample_relative_ts += duration_step;
            sample_idx += sample_step;
        }

        cr.stroke();

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
