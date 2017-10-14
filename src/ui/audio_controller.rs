extern crate cairo;

extern crate gtk;
use gtk::{Inhibit, ToolButtonExt, WidgetExt};

#[cfg(feature = "profiling-audio-draw")]
use chrono::Utc;

use std::boxed::Box;

use std::rc::Rc;
use std::cell::RefCell;

use std::sync::{Arc, Mutex};

use media::{Context, DoubleAudioBuffer, SampleExtractor, Timestamp};

use super::{BACKGROUND_COLOR, ControllerState, DoubleWaveformBuffer, MainController,
            WaveformConditions, WaveformBuffer};

const BUFFER_DURATION:   u64 = 60_000_000_000;    // 60 s
const MIN_REQ_DURATION:  f64 =      1_953_125f64; //  2 ms / 1000 px
const MAX_REQ_DURATION:  f64 = 32_000_000_000f64; // 32 s  / 1000 px
const INIT_REQ_DURATION: f64 =  4_000_000_000f64; //  4 s  / 1000 px
const STEP_REQ_DURATION: f64 =  2f64;

pub struct AudioController {
    container: gtk::Container,
    drawingarea: gtk::DrawingArea,
    zoom_in_btn: gtk::ToolButton,
    zoom_out_btn: gtk::ToolButton,

    is_active: bool,

    requested_duration: Rc<RefCell<f64>>,

    waveform_mtx: Arc<Mutex<Box<SampleExtractor>>>,
    dbl_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Self {
        let dbl_buffer_mtx = DoubleWaveformBuffer::new(BUFFER_DURATION);
        let waveform_mtx = dbl_buffer_mtx.lock()
            .expect("AudioController::new: couldn't lock dbl_buffer_mtx")
            .get_exposed_buffer_mtx();

        AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
            zoom_in_btn: builder.get_object("audio_zoom_in-toolbutton").unwrap(),
            zoom_out_btn: builder.get_object("audio_zoom_out-toolbutton").unwrap(),

            is_active: false,
            requested_duration: Rc::new(RefCell::new(INIT_REQ_DURATION)),
            waveform_mtx: waveform_mtx,
            dbl_buffer_mtx: dbl_buffer_mtx,
        }
    }

    pub fn register_callbacks(&self, main_ctrl: &Rc<RefCell<MainController>>) {
        // draw
        let waveform_mtx = Arc::clone(&self.waveform_mtx);
        let requested_duration = Rc::clone(&self.requested_duration);
        self.drawingarea.connect_draw(move |drawing_area, cairo_ctx| {
            AudioController::draw(
                drawing_area,
                cairo_ctx,
                &waveform_mtx,
                *requested_duration.borrow(),
            ).into()
        });

        // widget size changed
        let requested_duration = Rc::clone(&self.requested_duration);
        let dbl_buffer_mtx = Arc::clone(&self.dbl_buffer_mtx);
        self.drawingarea.connect_size_allocate(move |drawingarea, _| {
            AudioController::refresh(
                drawingarea,
                *requested_duration.borrow(),
                &dbl_buffer_mtx,
            );
        });

        // click in drawing_area
        let main_ctrl_rc = Rc::clone(&main_ctrl);
        let waveform_mtx = Arc::clone(&self.waveform_mtx);
        self.drawingarea.connect_button_press_event(move |_, event_button| {
            let button = event_button.get_button();
            if button == 1 {
                if let Some(position) = {
                    let waveform_buffer_grd = &mut *waveform_mtx.lock()
                        .expect("Couldn't lock waveform buffer in audio controller draw");
                    waveform_buffer_grd
                        .as_mut_any().downcast_mut::<WaveformBuffer>()
                        .expect("SamplesExtratctor is not a waveform buffer in audio controller draw")
                        .get_position(event_button.get_position().0)
                }
                {
                    main_ctrl_rc.borrow_mut().seek(position, true); // accurate (slow)
                }
            }
            Inhibit(true)
        });

        // click zoom in
        let drawingarea = self.drawingarea.clone();
        let requested_duration = Rc::clone(&self.requested_duration);
        let dbl_buffer_mtx = Arc::clone(&self.dbl_buffer_mtx);
        self.zoom_in_btn.connect_clicked(move |_| {
            let (can_update, duration) = {
                let mut duration_grd = requested_duration.borrow_mut();
                *duration_grd /= STEP_REQ_DURATION;
                if *duration_grd >= MIN_REQ_DURATION {
                    (true, *duration_grd)
                } else {
                    *duration_grd = MIN_REQ_DURATION;
                    (false, *duration_grd)
                }
            };

            if can_update {
                AudioController::refresh(
                    &drawingarea,
                    duration,
                    &dbl_buffer_mtx,
                );
            }
        });

        // click zoom out
        let drawingarea = self.drawingarea.clone();
        let requested_duration = Rc::clone(&self.requested_duration);
        let dbl_buffer_mtx = Arc::clone(&self.dbl_buffer_mtx);
        self.zoom_out_btn.connect_clicked(move |_| {
            let (can_update, duration) = {
                let mut duration_grd = requested_duration.borrow_mut();
                *duration_grd *= STEP_REQ_DURATION;
                if *duration_grd <= MAX_REQ_DURATION {
                    (true, *duration_grd)
                } else {
                    *duration_grd = MAX_REQ_DURATION;
                    (false, *duration_grd)
                }
            };

            if can_update {
                AudioController::refresh(
                    &drawingarea,
                    duration,
                    &dbl_buffer_mtx,
                );
            }
        });
    }

    pub fn cleanup(&mut self) {
        self.is_active = false;
        {
            self.dbl_buffer_mtx.lock()
                .expect("AudioController::cleanup: Couldn't lock dbl_buffer_mtx")
                .cleanup();
        }
        *self.requested_duration.borrow_mut() = INIT_REQ_DURATION;
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
            self.container.show();
        } else {
            self.container.hide();
        }
    }

    pub fn seek(&mut self, position: u64, state: &ControllerState) {
        {
            self.waveform_mtx.lock()
                .expect("AudioController::seek: Couldn't lock waveform_mtx")
                .as_mut_any().downcast_mut::<WaveformBuffer>()
                .expect("AudioController::seek: SamplesExtratctor is not a WaveformBuffer")
                .seek(position, *state == ControllerState::Playing);
        }

        if *state == ControllerState::Paused {
            // refresh the buffer in order to render the waveform
            // with samples that might not be rendered in current WaveformImage yet
            self.dbl_buffer_mtx.lock()
                .expect("AudioController::seek: couldn't lock dbl_buffer_mtx")
                .refresh();
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
        requested_duration: f64,
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

        let (first_pos, last_opt, cursor_opt) = {
            let waveform_grd = &mut *waveform_mtx.lock()
                .expect("AudioController::draw: couldn't lock waveform_mtx");
            let waveform_buffer = waveform_grd
                .as_mut_any().downcast_mut::<WaveformBuffer>()
                .expect("AudioController::draw: SamplesExtratctor is not a WaveformBuffer");

            #[cfg(feature = "profiling-audio-draw")]
            let _before_cndt = Utc::now();

            waveform_buffer.update_conditions(
                requested_duration,
                allocation.width,
                allocation.height
            );

            match waveform_buffer.get_image() {
                Some((image, x_offset, first_pos, last_opt, cursor_opt)) => {
                    #[cfg(feature = "profiling-audio-draw")]
                    let _before_image = Utc::now();

                    cr.set_source_surface(image, -x_offset, 0f64);
                    cr.paint();

                    (first_pos, last_opt, cursor_opt)
                },
                None => {
                    AudioController::clean_cairo_context(cr);
                    return Inhibit(true)
                },
            }
        };

        #[cfg(feature = "profiling-audio-draw")]
        let before_pos = Utc::now();

        let height = f64::from(allocation.height);
        let width = f64::from(allocation.width);
        cr.scale(1f64, 1f64);
        cr.set_source_rgb(1f64, 1f64, 0f64);
        cr.set_font_size(14f64);

        // first position
        let first_text = format!("{}", Timestamp::format(first_pos, false));
        let first_text_width = 2f64 + cr.text_extents(&first_text).width;
        cr.move_to(2f64, 30f64);
        cr.show_text(&first_text);

        // last position
        if let Some((last_x, last_pos)) = last_opt {
            let last_text = format!("{}", Timestamp::format(last_pos, false));
            // align to a 5px multiple box in order to avoid flickering
            let last_text_width =
                (cr.text_extents(&last_text).width / 5f64).ceil() * 5f64;
            if last_x + last_text_width > first_text_width + 10f64 {
                // last text won't overlap with first text
                cr.move_to(last_x - last_text_width, 30f64);
                cr.show_text(&last_text);
            }
        }

        if let Some((current_x, current_pos)) = cursor_opt {
            // draw current pos
            let cursor_text = format!("{}", Timestamp::format(current_pos, true));
            let cursor_text_width = 5f64 + cr.text_extents(&cursor_text).width;
            let cursor_text_x =
                if current_x + cursor_text_width < width {
                    current_x + 5f64
                } else {
                    current_x - cursor_text_width
                };
            cr.move_to(cursor_text_x, 15f64);
            cr.show_text(&cursor_text);

            cr.set_line_width(1f64);
            cr.move_to(current_x, 0f64);
            cr.line_to(current_x, height);
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

    fn refresh(
        drawingarea: &gtk::DrawingArea,
        requested_duration: f64,
        dbl_buffer_mtx: &Arc<Mutex<DoubleAudioBuffer>>,
    ) {
        let allocation = drawingarea.get_allocation();
        {
            // refresh the buffer in order to render the waveform
            // in latest conditions
            dbl_buffer_mtx.lock()
                .expect("AudioController::size-allocate: couldn't lock dbl_buffer_mtx")
                .refresh_with_conditions(
                    Box::new(WaveformConditions::new(
                        requested_duration,
                        allocation.width,
                        allocation.height
                    ))
                );
        }

        drawingarea.queue_draw();
    }
}
