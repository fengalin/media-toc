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

    stream_duration: u64,
    sample_duration: u64,
    stream_initialized: bool,

    sample_buffer: VecDeque<f64>,
    ts_offset: u64,
    ts_relative_offset: u64,
    buffer_duration: u64,
    has_received_last_buffer: bool,

    ts_relative_pos: u64,
    sample_skip_step: u64,
    sample_per_step: u64,
    iter_since_adjust: usize,
    min_iter_before_adjust: usize,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            stream_duration: 0,
            sample_duration: 0,
            stream_initialized: false,

            sample_buffer: VecDeque::with_capacity(12 * 48_000), // 12s @ 48kHz
            ts_offset: 0,
            ts_relative_offset: 0,
            buffer_duration: 0,
            has_received_last_buffer: false,

            ts_relative_pos: 0,
            sample_skip_step: 0,
            sample_per_step: 0,
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
            let duration = ctx.get_duration();
            if duration.is_negative() {
                panic!("Audio controller found a negative stream duration");
            }
            self.stream_duration = duration as u64;
            let position = ctx.get_position();
            if position.is_negative() {
                panic!("Audio controller found a negative initial position");
            }
            self.ts_offset = position as u64;
            self.stream_initialized = true;
            self.drawingarea.queue_draw();
            self.container.show();
        }
        else {
            self.container.hide();
        }
    }

    pub fn have_buffer(&mut self, mut buffer: AudioBuffer) {
        // assume the buffers come in order
        if self.sample_buffer.is_empty() {
            self.ts_offset = 0;
            self.ts_relative_offset = 0;
            self.buffer_duration = 0;
            self.has_received_last_buffer = false;

            self.ts_relative_pos = 0;
            self.sample_skip_step = 1;
            self.sample_per_step = 1;
            self.iter_since_adjust = 0;

            self.sample_duration = buffer.caps.sample_duration;
        }

        if buffer.pts < self.ts_offset {
            panic!("unordered buffers");
        }

        let mut samples_vecdeque = VecDeque::from_iter(buffer.samples.drain(..));
        self.sample_buffer.append(&mut samples_vecdeque);

        self.buffer_duration = self.sample_buffer.len() as u64 * self.sample_duration;
        self.has_received_last_buffer = self.stream_initialized
            && self.ts_relative_offset + self.buffer_duration >= self.stream_duration;

        if !self.has_received_last_buffer && self.buffer_duration > 10_000_000_000
            && self.ts_relative_pos > 3_000_000_000
        { // buffer larger than 10s and current pos is sufficient to:
            // remove 2s worse of samples
            // can't purge big chunks otherwise it impacts drawing
            // need to comply with current sample_per_step in order to
            // avoid discontinuities during waveform rendering
            // TODO: make sure if we actually loose decimal part
            let sample_nb_to_remove =
                (2_000_000_000 / self.sample_duration / self.sample_per_step)
                * self.sample_per_step;
            self.sample_buffer.drain(..(sample_nb_to_remove as usize));

            let duration_removed = sample_nb_to_remove * self.sample_duration;
            self.buffer_duration -= duration_removed;
            self.ts_offset += duration_removed;
            self.ts_relative_offset += duration_removed;
            self.ts_relative_pos -= duration_removed;
            //println!("buffer duration: {}", self.buffer_duration);
        }
    }

    pub fn have_position(&mut self, position: i64) {
        if position.is_negative() {
            panic!("Audio controller received a negative position");
        }
        if (position as u64) < self.ts_offset {
            panic!("Audio controller received a position earlier than first offset position: {}, offset {}", position, self.ts_offset);
        }
        let ts_relative_pos = position as u64 - self.ts_offset;

        if self.ts_relative_pos != ts_relative_pos {
            self.ts_relative_pos = ts_relative_pos;
            self.drawingarea.queue_draw();
        }
    }

    pub fn force_redraw(&mut self) {
        self.sample_skip_step = 1;
        self.iter_since_adjust = 0;
        self.drawingarea.queue_draw();
    }

    fn draw(&mut self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) -> Inhibit {
        if self.sample_buffer.is_empty() {
            return Inhibit(false);
        }
        let allocation = drawing_area.get_allocation();
        let width = allocation.width;
        if width.is_negative() {
            return Inhibit(false);
        }
        let width = width as u64;

        let display_window_duration = 2_000_000_000; // 2s TODO: adapt according to zoom
        let diplay_window_sample_nb = display_window_duration / self.sample_duration;

        let display_duration = self.buffer_duration.min(display_window_duration);
        let half_duration = display_duration / 2;
        let quarter_duration = half_duration / 2;
        //let three_quarter_duration = quarter_duration + half_duration;
        let diplay_sample_nb = display_duration / self.sample_duration;

        // TODO: make width and height adapt to zoom
        cr.scale((width / 2) as f64, (allocation.height / 2) as f64);
        cr.set_line_width(0.0015f64);

        // Take this opportunity to adjust sample skip step in order
        // to accomodate to computation requirements
        self.iter_since_adjust += 1;
        let first_display_pos = if
            !self.has_received_last_buffer && self.ts_relative_pos > self.buffer_duration
        {
            self.sample_skip_step += 1;
            self.iter_since_adjust = 0;
            self.buffer_duration - display_duration
        } else if self.ts_relative_pos > self.buffer_duration - half_duration {
            if !self.has_received_last_buffer && self.iter_since_adjust > self.min_iter_before_adjust
                && self.ts_relative_pos > self.buffer_duration - quarter_duration
            {
                self.sample_skip_step += 1;
                self.iter_since_adjust = 0;
            }
            self.buffer_duration - display_duration
        } else if self.ts_relative_pos > half_duration {
            // FIXME: need a real algorithm to stabilize
            /*if self.iter_since_adjust > self.min_iter_before_adjust
                && self.sample_skip_step > 1 && self.relative_pos < self.buffer_duration - three_quarter_duration
            {
                self.sample_skip_step -= 1;
                self.iter_since_adjust = 0;
                self.min_iter_before_adjust *= 2;
            }*/
            self.ts_relative_pos - half_duration
        } else {
            0
        };

        self.sample_per_step = if diplay_window_sample_nb > width / self.sample_skip_step {
            diplay_window_sample_nb / width * self.sample_skip_step
        } else {
            1
        };

        // Define the first sample as a multiple of self.sample_per_step
        // In order to avoid flickering when origin shifts between redraws
        let first_sample = (
            (first_display_pos / self.sample_duration / self.sample_per_step)
            * self.sample_per_step
        ) as usize;
        let last_sample = first_sample + diplay_sample_nb as usize;
        let sample_step = self.sample_per_step as usize;

        cr.set_source_rgb(0.8f64, 0.8f64, 0.8f64);

        let mut sample_relative_ts = 0f64;
        let duration_step = (self.sample_per_step * self.sample_duration) as f64
            / 1_000_000_000f64;

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
        let x = (self.ts_relative_pos - first_display_pos) as f64 / 1_000_000_000f64;
        cr.set_source_rgb(1f64, 1f64, 0f64);
        cr.set_line_width(0.004f64);
        cr.move_to(x, 0f64);
        cr.line_to(x, 2f64);
        cr.stroke();

        Inhibit(false)
    }
}
