extern crate glib;

extern crate gtk;
use gtk::{Inhibit, WidgetExt};

extern crate cairo;

use std::rc::Rc;
use std::cell::RefCell;

use std::sync::{Arc, Mutex};

use ::media::{Context, WaveformBuffer};

pub struct AudioController {
    container: gtk::Container,
    drawingarea: gtk::DrawingArea,

    position: u64,
    waveform_buffer_mtx: Arc<Mutex<Option<WaveformBuffer>>>,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            position: 0,
            waveform_buffer_mtx: Arc::new(Mutex::new(None)),
        }));

        {
            let this_ref = this.borrow();
            let this_rc = this.clone();
            this_ref.drawingarea.connect_draw(move |drawing_area, cairo_ctx| {
                this_rc.borrow()
                    .draw(drawing_area, cairo_ctx).into()
            });
        }

        this
    }

    pub fn cleanup(&mut self) {
        // force redraw to purge the double buffer
        self.drawingarea.queue_draw();
    }

    pub fn new_media(&mut self, context: &Context) {
        let has_audio = context.info.lock()
                .expect("Failed to lock media info while initializing audio controller")
                .audio_best
                .is_some();

        if has_audio {
            self.position = 0;
            self.waveform_buffer_mtx = context.waveform_buffer_mtx.clone();

            self.container.show();
        }
        else {
            self.container.hide();
        }
    }

    pub fn tic(&mut self, position: u64) {
        self.position = position;
        self.drawingarea.queue_draw();
    }

    fn draw(&self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) -> Inhibit {
        if self.position == 0 {
            return Inhibit(false);
        }

        let allocation = drawing_area.get_allocation();
        let width = allocation.width;
        if width.is_negative() {
            return Inhibit(false);
        }

        let requested_duration = 2_000_000_000u64; // 2s TODO: adapt according to zoom

        // TODO: make width and height adapt to zoom
        let width = width as u64;
        cr.scale((width / 2) as f64, (allocation.height / 2) as f64);

        cr.set_line_width(0.0015f64);
        cr.set_source_rgb(0.8f64, 0.8f64, 0.8f64);

        // resolution
        let requested_step_duration =
            if requested_duration > width {
                requested_duration / width
            } else {
                1
            };

        let first_visible_pts = {
            let mut waveform_buffer_opt = self.waveform_buffer_mtx.lock()
                .expect("Couldn't lock waveform buffer in audio controller draw");
            let waveform_buffer =
                match waveform_buffer_opt.as_mut() {
                    Some(waveform_buffer) => waveform_buffer,
                    None => return Inhibit(false),
                };

            waveform_buffer.update_conditions(
                self.position,
                requested_duration,
                requested_step_duration
            );

            if waveform_buffer.samples.is_empty() {
                return Inhibit(false);
            }

            let mut relative_pts = 0f64;
            let step_duration = waveform_buffer.step_duration / 1_000_000_000f64;

            let mut sample_iter = waveform_buffer.iter();
            cr.move_to(relative_pts, *sample_iter.next().unwrap());

            for sample in sample_iter {
                relative_pts += step_duration;
                cr.line_to(relative_pts, *sample);
            }

            waveform_buffer.first_visible_pts
        };

        cr.stroke();

        // draw current pos
        let x = (self.position as f64 - first_visible_pts) / 1_000_000_000f64;
        cr.set_source_rgb(1f64, 1f64, 0f64);
        cr.set_line_width(0.004f64);
        cr.move_to(x, 0f64);
        cr.line_to(x, 2f64);
        cr.stroke();

        Inhibit(false)
    }
}
