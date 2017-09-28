extern crate cairo;

use glib::ObjectExt;

extern crate gtk;
use gtk::{Inhibit, WidgetExt};

#[cfg(feature = "profiling-audio-draw")]
use chrono::Utc;

use std::boxed::Box;

use std::rc::Rc;
use std::cell::RefCell;

use std::sync::{Arc, Mutex};

use media::{Context, DoubleAudioBuffer, SampleExtractor};

use super::{BACKGROUND_COLOR, DoubleWaveformBuffer, MainController, WaveformBuffer};

const INIT_REQ_DURATION: u64 = 2_000_000_000; // 2s

pub struct AudioController {
    container: gtk::Container,
    pub drawingarea: gtk::DrawingArea,

    is_active: bool,
    position: u64,

    // need Arc<Mutex> here until connect_size_allocate is implemented in gtk-rs
    requested_duration_mtx: Arc<Mutex<u64>>,

    waveform_mtx: Arc<Mutex<Box<SampleExtractor>>>,
    dbl_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Self {
        let dbl_buffer_mtx =
            DoubleWaveformBuffer::new(
                //30_000_000_000 // 30s buffer_duration
                10_000_000_000 // 10s buffer_duration
            );
        let waveform_mtx = dbl_buffer_mtx.lock()
            .expect("AudioController::new: couldn't lock dbl_buffer_mtx")
            .get_exposed_buffer_mtx();

        AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),

            is_active: false,
            position: 0,
            requested_duration_mtx: Arc::new(Mutex::new(INIT_REQ_DURATION)),
            waveform_mtx: waveform_mtx,
            dbl_buffer_mtx: dbl_buffer_mtx,
        }
    }

    pub fn register_callbacks(&self, main_ctrl: &Rc<RefCell<MainController>>) {
        // draw
        let waveform_mtx = Arc::clone(&self.waveform_mtx);
        let requested_duration_mtx = Arc::clone(&self.requested_duration_mtx);
        self.drawingarea.connect_draw(move |drawing_area, cairo_ctx| {
            AudioController::draw(
                drawing_area,
                cairo_ctx,
                &waveform_mtx,
                &requested_duration_mtx,
            ).into()
        });

        // widget size changed
        let waveform_mtx = Arc::clone(&self.waveform_mtx);
        let requested_duration_mtx = Arc::clone(&self.requested_duration_mtx);
        let dbl_buffer_mtx = Arc::clone(&self.dbl_buffer_mtx);
        self.drawingarea.connect("size-allocate", false, move |drawingarea| {
            AudioController::on_size_allocate(
                drawingarea[0].get::<gtk::DrawingArea>().unwrap(),
                &waveform_mtx,
                &requested_duration_mtx,
                &dbl_buffer_mtx,
            );
            None
        }).ok().unwrap();

        // click in drawing_area
        let main_ctrl_rc = Rc::clone(main_ctrl);
        let waveform_mtx = Arc::clone(&self.waveform_mtx);
        self.drawingarea.connect_button_press_event(move |_, event_button| {
            if event_button.get_button() == 1 {
                if let Some(position) = {
                    let waveform_buffer_grd = &mut *waveform_mtx.lock()
                        .expect("Couldn't lock waveform buffer in audio controller draw");
                    waveform_buffer_grd
                        .as_mut_any().downcast_mut::<WaveformBuffer>()
                        .expect("SamplesExtratctor is not a waveform buffer in audio controller draw")
                        .seek_in_window(event_button.get_position().0)
                }
                {
                    main_ctrl_rc.borrow_mut().seek(position);
                }
            }
            Inhibit(true)
        });
    }

    pub fn cleanup(&mut self) {
        self.is_active = false;
        {
            self.dbl_buffer_mtx.lock()
                .expect("AudioController::cleanup: Couldn't lock dbl_buffer_mtx")
                .cleanup();
        }
        self.drawingarea.queue_draw();
    }

    pub fn get_dbl_buffer_mtx(&self) -> Arc<Mutex<DoubleAudioBuffer>> {
        Arc::clone(&self.dbl_buffer_mtx)
    }

    pub fn new_media(&mut self, context: &Context) {
        let has_audio = context.info.lock()
                .expect("Failed to lock media info while initializing audio controller")
                .audio_best
                .is_some();

        if has_audio {
            self.is_active = true;
            self.position = 0;

            self.container.show();
        } else {
            self.container.hide();
        }
    }

    pub fn seek(&mut self, position: u64) {
        {
            self.waveform_mtx.lock()
                .expect("AudioController::seek: Couldn't lock waveform_mtx")
                .as_mut_any().downcast_mut::<WaveformBuffer>()
                .expect("AudioController::seek: SamplesExtratctor is not a WaveformBuffer")
                .seek(position);
        }
        self.tick();
    }

    pub fn tick(&self) {
        if self.is_active {
            self.drawingarea.queue_draw();
        }
    }

    fn clean_cairo_context(cr: &cairo::Context) {
        cr.set_source_rgb(
            BACKGROUND_COLOR.0,
            BACKGROUND_COLOR.1,
            BACKGROUND_COLOR.2
        );
        cr.paint();
    }

    fn draw(
        drawingarea: &gtk::DrawingArea,
        cr: &cairo::Context,
        waveform_mtx: &Arc<Mutex<Box<SampleExtractor>>>,
        requested_duration_mtx: &Arc<Mutex<u64>>,
    ) -> Inhibit {
        #[cfg(feature = "profiling-audio-draw")]
        let before_init = Utc::now();

        let allocation = drawingarea.get_allocation();
        if allocation.width.is_negative() {
            println!("negative allocation.width: {}", allocation.width);
            return Inhibit(true);
        }

        #[cfg(feature = "profiling-audio-draw")]
        let before_lock = Utc::now();
        #[cfg(feature = "profiling-audio-draw")]
        let mut _before_cndt = Utc::now();
        #[cfg(feature = "profiling-audio-draw")]
        let mut _before_image = Utc::now();

        let current_x_opt = {
            let waveform_grd = &mut *waveform_mtx.lock()
                .expect("AudioController::draw: couldn't lock waveform_mtx");
            let waveform_buffer = waveform_grd
                .as_mut_any().downcast_mut::<WaveformBuffer>()
                .expect("AudioController::draw: SamplesExtratctor is not a WaveformBuffer");

            #[cfg(feature = "profiling-audio-draw")]
            let _before_cndt = Utc::now();

            waveform_buffer.update_condition(
                *requested_duration_mtx.lock().unwrap(),
                allocation.width,
                allocation.height
            );

            match waveform_buffer.get_image() {
                Some((image, x_offset, current_x_opt)) => {
                    #[cfg(feature = "profiling-audio-draw")]
                    let _before_image = Utc::now();

                    cr.set_source_surface(image, -x_offset, 0f64);
                    cr.paint();

                    current_x_opt
                },
                None => {
                    AudioController::clean_cairo_context(cr);
                    return Inhibit(true)
                },
            }
        };

        #[cfg(feature = "profiling-audio-draw")]
        let before_pos = Utc::now();

        if let Some(current_x) = current_x_opt {
            // draw current pos
            cr.scale(1f64, f64::from(allocation.height));
            cr.set_source_rgb(1f64, 1f64, 0f64);
            cr.set_line_width(1f64);
            cr.move_to(current_x, 0f64);
            cr.line_to(current_x, 1f64);
            cr.stroke();
        }

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

       Inhibit(true)
    }

    fn on_size_allocate(
        drawingarea: gtk::DrawingArea,
        waveform_mtx: &Arc<Mutex<Box<SampleExtractor>>>,
        requested_duration_mtx: &Arc<Mutex<u64>>,
        dbl_buffer_mtx: &Arc<Mutex<DoubleAudioBuffer>>,
    ) {
        let allocation = drawingarea.get_allocation();
        let need_update = {
            let waveform_grd = &mut *waveform_mtx.lock()
                .expect("AudioController::draw: couldn't lock waveform_mtx");
            waveform_grd
                .as_mut_any().downcast_mut::<WaveformBuffer>()
                .expect("AudioController::draw: SamplesExtratctor is not a WaveformBuffer")
                .update_condition(
                    *requested_duration_mtx.lock().unwrap(),
                    allocation.width,
                    allocation.height
                )
        };

        if need_update {
            // refresh the buffer in order to render the waveform
            // in latest conditions
            dbl_buffer_mtx.lock()
                .expect("AudioController::size-allocate: couldn't lock dbl_buffer_mtx")
                .refresh();
            drawingarea.queue_draw();
        }
    }
}
