extern crate glib;

extern crate gtk;
use gtk::{Inhibit, WidgetExt};

extern crate cairo;

use std::rc::Rc;
use std::cell::RefCell;

use std::sync::{Arc, Mutex};

use ::media::{Context, AudioBuffer};

pub struct AudioController {
    container: gtk::Container,
    drawingarea: gtk::DrawingArea,

    audio_buffer: Arc<Mutex<AudioBuffer>>,
    can_draw: bool,
    position: u64,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            audio_buffer: Arc::new(Mutex::new(AudioBuffer::new())),
            can_draw: false,
            position: 0,
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

    pub fn cleanup(&mut self) {
        self.can_draw = false;
        self.position = 0;
    }

    pub fn new_media(&mut self, ctx: &Context) {
        let has_audio = {
            let ctx_info = ctx.info.lock()
                .expect("Failed to lock media info while initializing audio controller");
            ctx_info.audio_best.is_some()
        };

        if has_audio {
            let position = ctx.get_position();
            if position.is_negative() {
                panic!("Audio controller found a negative initial position");
            }
            self.position = position as u64;

            self.audio_buffer = ctx.audio_buffer.clone();
            {
                let audio_buffer = &mut self.audio_buffer.lock()
                    .expect("Failed to lock audio buffer while initializing audio controller");
                audio_buffer.set_first_pts(self.position);
            }

            self.can_draw = true;
            self.drawingarea.queue_draw();
            self.container.show();
        }
        else {
            self.container.hide();
        }
    }

    pub fn have_position(&mut self, position: i64) {
        if position.is_negative() {
            panic!("Audio controller received a negative position");
        }

        let position = position as u64;

        if self.position != position {
            self.position = position;
            self.drawingarea.queue_draw();
        }
    }

    fn draw(&mut self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) -> Inhibit {
        if !self.can_draw {
            return Inhibit(false);
        }
        let allocation = drawing_area.get_allocation();
        let width = allocation.width;
        if width.is_negative() {
            return Inhibit(false);
        }
        let width = width as u64;

        let display_window_duration = 2_000_000_000; // 2s TODO: adapt according to zoom

        // TODO: make width and height adapt to zoom
        cr.scale((width / 2) as f64, (allocation.height / 2) as f64);
        cr.set_line_width(0.0015f64);
        cr.set_source_rgb(0.8f64, 0.8f64, 0.8f64);

        let first_display_pos = {
            let audio_buffer = &mut self.audio_buffer.lock()
                .expect("Couldn't lock audio buffer in audio controller draw");
            audio_buffer.set_pts(self.position);

            if audio_buffer.duration == 0 {
                return Inhibit(false);
            }

            let diplay_window_sample_nb = display_window_duration / audio_buffer.sample_duration;
            let sample_per_pixel =
                if diplay_window_sample_nb > width {
                    diplay_window_sample_nb / width
                } else {
                    1
                };

            let pixel_duration = sample_per_pixel * audio_buffer.sample_duration;

            let (first_display_pos, display_duration) =
                if audio_buffer.duration < display_window_duration {
                    (0, audio_buffer.duration)
                } else {
                    let half_duration = display_window_duration / 2;
                    let first_display_pos =
                        if self.position > audio_buffer.last_pts - half_duration {
                            audio_buffer.last_pts - display_window_duration
                        } else if self.position > audio_buffer.first_pts + half_duration {
                            self.position - half_duration
                        } else {
                            0
                        };
                    (first_display_pos, display_window_duration)
                };

            // align first display pos as a multiple of pixel duration
            // in order to avoid discontinuities when origin shifts between redraws
            let aligned_first_display_pos = (first_display_pos / pixel_duration) * pixel_duration;

            let first_sample =
                (aligned_first_display_pos / audio_buffer.sample_duration) as usize
                - audio_buffer.first_sample_offset;

            let sample_step = sample_per_pixel as usize;
            let last_sample =
                first_sample
                + (display_duration / audio_buffer.sample_duration) as usize
                + sample_step; // add one step to compensate the offset due to aligned_first_display_pos
            let last_sample = last_sample.min(audio_buffer.samples.len());

            let mut relative_pts =
                (0f64 - ((first_display_pos - aligned_first_display_pos) as f64))
                 / 1_000_000_000f64;
            let duration_step = pixel_duration as f64 / 1_000_000_000f64;

            let mut sample_idx = first_sample;

            cr.move_to(relative_pts, audio_buffer.samples[sample_idx]);
            relative_pts += duration_step;
            sample_idx += sample_step;

            while sample_idx < last_sample {
                cr.line_to(relative_pts, audio_buffer.samples[sample_idx]);
                relative_pts += duration_step;
                sample_idx += sample_step;
            }

            first_display_pos
        };

        cr.stroke();

        // draw current pos
        let x = (self.position - first_display_pos) as f64 / 1_000_000_000f64;
        cr.set_source_rgb(1f64, 1f64, 0f64);
        cr.set_line_width(0.004f64);
        cr.move_to(x, 0f64);
        cr.line_to(x, 2f64);
        cr.stroke();

        Inhibit(false)
    }
}
