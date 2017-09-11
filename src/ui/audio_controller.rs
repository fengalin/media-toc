extern crate gtk;
use gtk::{Inhibit, WidgetExt};

extern crate cairo;

#[cfg(feature = "profiling-audio-draw")]
use chrono::Utc;

use std::boxed::Box;

use std::rc::Rc;
use std::cell::RefCell;

use std::sync::{Arc, Mutex};

use ::media::{Context, SamplesExtractor};

use super::WaveformBuffer;

pub struct AudioController {
    container: gtk::Container,
    drawingarea: gtk::DrawingArea,

    is_active: bool,
    position: u64,
    samples_extractor_mtx: Arc<Mutex<Box<SamplesExtractor>>>,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            is_active: false,
            position: 0,
            samples_extractor_mtx: Arc::new(Mutex::new(Box::new(WaveformBuffer::new()))),
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
        self.is_active = false;
        // force redraw to purge the double buffer
        self.drawingarea.queue_draw();
    }

    pub fn new_media(&mut self, context: &Context) {
        let has_audio = context.info.lock()
                .expect("Failed to lock media info while initializing audio controller")
                .audio_best
                .is_some();

        if has_audio {
            self.is_active = true;
            self.position = 0;
            self.samples_extractor_mtx = context.samples_extractor_mtx.clone();

            self.container.show();
        } else {
            self.container.hide();
        }
    }

    pub fn tic(&mut self, position: u64) {
        self.position = position;
        self.drawingarea.queue_draw();
    }

    fn draw(&self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) -> Inhibit {
        if !self.is_active {
            return Inhibit(false);
        }

        #[cfg(feature = "profiling-audio-draw")]
        let before_init = Utc::now();

        if self.position == 0 {
            return Inhibit(false);
        }

        let allocation = drawing_area.get_allocation();
        if allocation.width.is_negative() {
            return Inhibit(false);
        }

        let requested_duration = 2_000_000_000u64; // 2s

        #[cfg(feature = "profiling-audio-draw")]
        let before_lock = Utc::now();
        #[cfg(feature = "profiling-audio-draw")]
        let mut _before_cndt = Utc::now();
        #[cfg(feature = "profiling-audio-draw")]
        let mut _before_image = Utc::now();

        let current_x = {
            let waveform_buffer_grd = &mut *self.samples_extractor_mtx.lock()
                .expect("Couldn't lock waveform buffer in audio controller draw");
            let waveform_buffer = waveform_buffer_grd
                .as_mut_any().downcast_mut::<WaveformBuffer>()
                .expect("SamplesExtratctor is not a waveform buffer in audio controller draw");

            #[cfg(feature = "profiling-audio-draw")]
            let _before_cndt = Utc::now();

            waveform_buffer.update_conditions(
                    self.position,
                    requested_duration,
                    allocation.width,
                    allocation.height,
                );

            #[cfg(feature = "profiling-audio-draw")]
            let _before_image = Utc::now();

            let image = match waveform_buffer.image_surface.as_ref() {
                Some(image) => image,
                None => return Inhibit(false),
            };

            cr.set_source_surface(image, -(waveform_buffer.x_offset as f64), 0f64);
            cr.paint();

            waveform_buffer.current_x
        };

        #[cfg(feature = "profiling-audio-draw")]
        let before_pos = Utc::now();

        // draw current pos
        cr.scale(1f64, allocation.height as f64);
        cr.set_source_rgb(1f64, 1f64, 0f64);
        cr.set_line_width(1f64);
        let current_pos = current_x as f64;
        cr.move_to(current_pos, 0f64);
        cr.line_to(current_pos, 1f64);
        cr.stroke();

        #[cfg(feature = "profiling-audio-draw")]
        let end = Utc::now();

        #[cfg(feature = "profiling-audio-draw")]
        println!("audio-draw,{},{},{},{},{},{}",
            before_init.time().format("%H:%M:%S%.6f"),
            before_lock.time().format("%H:%M:%S%.6f"),
            _before_cndt.time().format("%H:%M:%S%.6f"),
            _before_image.time().format("%H:%M:%S%.6f"),
            before_pos.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
        );

       Inhibit(false)
    }
}
