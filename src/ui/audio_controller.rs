extern crate glib;

extern crate gtk;
use gtk::{Inhibit, WidgetExt};

extern crate cairo;

use std::rc::{Rc, Weak};
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use ::media::{Context, WaveformBuffer};

pub struct AudioController {
    container: gtk::Container,
    drawingarea: gtk::DrawingArea,

    context: Option<Weak<RefCell<Context>>>,
    waveform_buffer_mtx: Arc<Mutex<Option<WaveformBuffer>>>,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            context: None,
            waveform_buffer_mtx: Arc::new(Mutex::new(None)),
        }));

        {
            let this_ref = this.borrow();
            let this_weak = Rc::downgrade(&this);
            this_ref.drawingarea.connect_draw(move |drawing_area, cairo_ctx| {
                match this_weak.upgrade() {
                    Some(this_ref) => {
                        this_ref.borrow_mut()
                            .draw(drawing_area, cairo_ctx).into()
                    },
                    None => Inhibit(false),
                }
            });
        }

        this
    }

    pub fn cleanup(&mut self) {
        self.context = None;
        // force redraw to purge the double buffer
        self.drawingarea.queue_draw();
    }

    pub fn new_media(&mut self, context_rc: &Rc<RefCell<Context>>) {
        let context = context_rc.borrow();

        let has_audio = context.info.lock()
                .expect("Failed to lock media info while initializing audio controller")
                .audio_best
                .is_some();

        if has_audio {
            self.waveform_buffer_mtx = context.waveform_buffer_mtx.clone();
            self.context = Some(Rc::downgrade(context_rc));
            self.container.show();
        }
        else {
            self.container.hide();
        }
    }

    pub fn tic(&self) {
        self.drawingarea.queue_draw();
    }

    fn draw(&mut self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) -> Inhibit {
        let context_rc = {
            if let Some(context_weak) = self.context.as_ref() {
                if let Some(context_rc) = context_weak.upgrade() {
                    context_rc
                } else { return Inhibit(false); }
            } else { return Inhibit(false); }
        };

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

        // TODO: find another way to get the position as it results
        // in locks (e.g. use a channel and send it from the Context' inspector)
        let position = {
            let context = context_rc.borrow();
            let position = context.get_position();
            if position.is_negative() {
                return Inhibit(false);
            }
            position as u64
        };

        let first_visible_pts = {
            let mut waveform_buffer_opt = self.waveform_buffer_mtx.lock()
                .expect("Couldn't lock waveform buffer in audio controller draw");
            let waveform_buffer = waveform_buffer_opt.as_mut()
                .expect("No waveform buffer buffer in audio controller draw");

            waveform_buffer.update_conditions(
                position,
                requested_duration,
                requested_step_duration
            );

            if waveform_buffer.duration == 0 {
                return Inhibit(false);
            }

            let mut relative_pts =
                ((waveform_buffer.first_visible_pts - waveform_buffer.first_pts) as f64)
                 / -1_000_000_000f64;

            let step_duration =
                waveform_buffer.step_duration as f64
                 / 1_000_000_000f64;

            let mut sample_iter = waveform_buffer.samples.iter();
            cr.move_to(relative_pts, *sample_iter.next().unwrap());

            for sample in sample_iter {
                relative_pts += step_duration;
                cr.line_to(relative_pts, *sample);
            }

            waveform_buffer.first_visible_pts
        };

        cr.stroke();

        // draw current pos
        let x = (position - first_visible_pts) as f64 / 1_000_000_000f64;
        cr.set_source_rgb(1f64, 1f64, 0f64);
        cr.set_line_width(0.004f64);
        cr.move_to(x, 0f64);
        cr.line_to(x, 2f64);
        cr.stroke();

        Inhibit(false)
    }
}
