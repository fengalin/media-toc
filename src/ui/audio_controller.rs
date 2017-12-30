extern crate cairo;
extern crate glib;

extern crate gtk;
use gtk::{Inhibit, ToolButtonExt, WidgetExt, WidgetExtManual};

#[cfg(feature = "profiling-audio-draw")]
use chrono::Utc;

use std::boxed::Box;

use std::rc::Rc;
use std::cell::RefCell;

use std::sync::{Arc, Mutex};

use media::{PlaybackContext, DoubleAudioBuffer, SampleExtractor};

use toc::Timestamp;

use super::{ControllerState, DoubleWaveformBuffer, MainController, WaveformBuffer,
            WaveformConditions, BACKGROUND_COLOR};

const BUFFER_DURATION: u64 = 60_000_000_000; // 60 s
const MIN_REQ_DURATION: f64 = 1_953_125f64; // 2 ms / 1000 px
const MAX_REQ_DURATION: f64 = 32_000_000_000f64; // 32 s / 1000 px
const INIT_REQ_DURATION: f64 = 4_000_000_000f64; // 4 s / 1000 px
const STEP_REQ_DURATION: f64 = 2f64;

const MIN_RANGE_DURATION: u64 = 100_000_000; // 100 ms

pub struct AudioController {
    container: gtk::Box,
    drawingarea: gtk::DrawingArea,
    zoom_in_btn: gtk::ToolButton,
    zoom_out_btn: gtk::ToolButton,

    is_active: bool,
    playback_needs_refresh: bool,

    requested_duration: f64,
    current_position: u64,
    last_visible_pos: u64,

    waveform_mtx: Arc<Mutex<Box<SampleExtractor>>>,
    dbl_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,

    tick_cb_id: Option<u32>,
}

impl AudioController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let dbl_buffer_mtx = DoubleWaveformBuffer::new_mutex(BUFFER_DURATION);
        let waveform_mtx = dbl_buffer_mtx
            .lock()
            .expect("AudioController::new: couldn't lock dbl_buffer_mtx")
            .get_exposed_buffer_mtx();

        Rc::new(RefCell::new(AudioController {
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
            zoom_in_btn: builder.get_object("audio_zoom_in-toolbutton").unwrap(),
            zoom_out_btn: builder.get_object("audio_zoom_out-toolbutton").unwrap(),

            is_active: false,
            playback_needs_refresh: false,

            requested_duration: INIT_REQ_DURATION,
            current_position: 0,
            last_visible_pos: 0,

            waveform_mtx: waveform_mtx,
            dbl_buffer_mtx: dbl_buffer_mtx,

            tick_cb_id: None,
        }))
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let this = this_rc.borrow();

        // draw
        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.drawingarea.connect_draw(
            move |drawing_area, cairo_ctx| {
                this_clone.borrow_mut()
                    .draw(&main_ctrl_clone, drawing_area, cairo_ctx)
            },
        );

        // widget size changed
        let main_ctrl_clone = Rc::clone(main_ctrl);
        let this_clone = Rc::clone(this_rc);
        this.drawingarea.connect_size_allocate(move |_, _| {
            this_clone.borrow_mut().refresh(&main_ctrl_clone);
        });

        // click in drawing_area
        let main_ctrl_clone = Rc::clone(main_ctrl);
        let this_clone = Rc::clone(this_rc);
        this.drawingarea.connect_button_press_event(
            move |_, event_button| {
                match event_button.get_button() {
                    1 => { // left click => seek
                        let position_opt = this_clone.borrow()
                            .get_position_at(event_button.get_position().0);
                        if let Some(position) = position_opt {
                            main_ctrl_clone.borrow_mut().seek(position, true); // accurate (slow)
                        }
                    }
                    3 => { // right click => segment playback
                        let (position_opt, current_position, last_pos) = {
                            let this = this_clone.borrow();
                            (
                                this.get_position_at(event_button.get_position().0),
                                this.current_position,
                                this.last_visible_pos,
                            )
                        };
                        if let Some(start_pos) = position_opt {
                            // get a reasonable range so that we can still hear
                            // something even when there are few samples in current window
                            let end_pos = start_pos + MIN_RANGE_DURATION.max(last_pos - start_pos);
                            main_ctrl_clone.borrow_mut()
                                .play_range(start_pos, end_pos, current_position);
                        }
                    }
                    _ => (),
                }
                Inhibit(true)
            },
        );

        // click zoom in
        let main_ctrl_clone = Rc::clone(main_ctrl);
        let this_clone = Rc::clone(this_rc);
        this.zoom_in_btn.connect_clicked(move |_| {
            let mut this = this_clone.borrow_mut();
            this.requested_duration /= STEP_REQ_DURATION;
            if this.requested_duration >= MIN_REQ_DURATION {
                this.refresh(&main_ctrl_clone);
            } else {
                this.requested_duration = MIN_REQ_DURATION;
            }
        });

        // click zoom out
        let main_ctrl_clone = Rc::clone(main_ctrl);
        let this_clone = Rc::clone(this_rc);
        this.zoom_out_btn.connect_clicked(move |_| {
            let mut this = this_clone.borrow_mut();
            this.requested_duration *= STEP_REQ_DURATION;
            if this.requested_duration <= MAX_REQ_DURATION {
                this.refresh(&main_ctrl_clone);
            } else {
                this.requested_duration = MAX_REQ_DURATION;
            }
        });
    }

    pub fn redraw(&self) {
        self.drawingarea.queue_draw();
    }

    pub fn cleanup(&mut self) {
        self.is_active = false;
        self.playback_needs_refresh = false;
        {
            self.dbl_buffer_mtx
                .lock()
                .expect("AudioController::cleanup: Couldn't lock dbl_buffer_mtx")
                .cleanup();
        }
        self.requested_duration = INIT_REQ_DURATION;
        self.current_position = 0;
        self.last_visible_pos = 0;
        self.redraw();
    }

    pub fn get_dbl_buffer_mtx(&self) -> Arc<Mutex<DoubleAudioBuffer>> {
        Arc::clone(&self.dbl_buffer_mtx)
    }

    pub fn new_media(&mut self, context: &PlaybackContext) {
        let has_audio = context
            .info
            .lock()
            .expect(
                "Failed to lock media info while initializing audio controller",
            )
            .audio_best
            .is_some();

        if has_audio {
            let allocation = self.drawingarea.get_allocation();
            {
                // init the buffers in order to render the waveform in current conditions
                let requested_duration = self.requested_duration;
                self.dbl_buffer_mtx
                    .lock()
                    .expect(
                        "AudioController::size-allocate: couldn't lock dbl_buffer_mtx",
                    )
                    .set_conditions(Box::new(WaveformConditions::new(
                        requested_duration,
                        allocation.width,
                        allocation.height,
                    )));
            }

            self.is_active = true;
            self.container.show();
        } else {
            self.container.hide();
        }
    }

    pub fn seek(&mut self, position: u64, state: &ControllerState) {
        {
            self.waveform_mtx
                .lock()
                .expect("AudioController::seek: Couldn't lock waveform_mtx")
                .as_mut_any()
                .downcast_mut::<WaveformBuffer>()
                .expect(
                    "AudioController::seek: SamplesExtratctor is not a WaveformBuffer",
                )
                .seek(position, *state == ControllerState::Playing);
        }

        if *state == ControllerState::Paused {
            // refresh the buffer in order to render the waveform
            // with samples that might not be rendered in current WaveformImage yet
            self.dbl_buffer_mtx
                .lock()
                .expect("AudioController::seek: couldn't lock dbl_buffer_mtx")
                .refresh(false); // don't keep continuity
        }
        self.redraw();
    }

    pub fn start_play_range(&mut self) {
        self.waveform_mtx
            .lock()
            .expect("AudioController::seek: Couldn't lock waveform_mtx")
            .as_mut_any()
            .downcast_mut::<WaveformBuffer>()
            .expect(
                "AudioController::seek: SamplesExtratctor is not a WaveformBuffer",
            )
            .start_play_range();
    }

    pub fn remove_tick_callback(this_rc: &Rc<RefCell<Self>>) {
        let mut this = this_rc.borrow_mut();
        if let Some(tick_cb_id) = this.tick_cb_id.take() {
            this.drawingarea.remove_tick_callback(tick_cb_id);
        }
    }

    pub fn register_tick_callback(this_rc: &Rc<RefCell<Self>>) {
        let mut this = this_rc.borrow_mut();
        if this.tick_cb_id.is_some() {
            return;
        }

        let this_rc = Rc::clone(this_rc);
        this.tick_cb_id = Some(
            this.drawingarea.add_tick_callback(move |_da, _frame_clock| {
                let this = this_rc.borrow_mut();
                if this.is_active {
                    if this.playback_needs_refresh {
                        #[cfg(feature = "trace-audio-controller")]
                        println!("AudioController::tick forcing refresh");

                        this.dbl_buffer_mtx
                            .lock()
                            .expect("AudioController::tick: couldn't lock dbl_buffer_mtx")
                            .refresh(true); // keep continuity
                    }

                    this.redraw();
                }
                glib::Continue(true)
            })
        );
    }

    fn get_position_at(&self, x: f64) -> Option<u64> {
        let waveform_buffer_grd = &mut *self.waveform_mtx.lock().expect(
            "Couldn't lock waveform buffer in audio controller draw",
        );
        waveform_buffer_grd
            .as_any()
            .downcast_ref::<WaveformBuffer>()
            .expect(concat!(
                "AudioController::get_current_and_sample_at ",
                "SamplesExtratctor is not a waveform buffer",
            ))
            .get_position_at(x)
    }

    fn clean_cairo_context(&self, cr: &cairo::Context) {
        cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
        cr.paint();
    }

    fn draw(
        &mut self,
        main_ctrl: &Rc<RefCell<MainController>>,
        drawingarea: &gtk::DrawingArea,
        cr: &cairo::Context
    ) -> Inhibit {
        #[cfg(feature = "profiling-audio-draw")]
        let before_init = Utc::now();

        let allocation = drawingarea.get_allocation();
        if allocation.width.is_negative() {
            #[cfg(feature = "trace-audio-controller")]
            println!(
                "AudioController::draw negative allocation.width: {}",
                allocation.width
            );

            self.clean_cairo_context(cr);
            return Inhibit(true);
        }

        #[cfg(feature = "profiling-audio-draw")]
        let before_lock = Utc::now();
        #[cfg(feature = "profiling-audio-draw")]
        let mut _before_cndt = Utc::now();
        #[cfg(feature = "profiling-audio-draw")]
        let mut _before_image = Utc::now();

        let (current_position, image_positions) = {
            let waveform_grd = &mut *self.waveform_mtx.lock().expect(
                "AudioController::draw: couldn't lock waveform_mtx",
            );
            let waveform_buffer = waveform_grd
                .as_mut_any()
                .downcast_mut::<WaveformBuffer>()
                .expect(
                    "AudioController::draw: SamplesExtratctor is not a WaveformBuffer",
                );

            #[cfg(feature = "profiling-audio-draw")]
            let _before_cndt = Utc::now();

            waveform_buffer.update_conditions(
                self.requested_duration,
                allocation.width,
                allocation.height,
            );

            self.playback_needs_refresh = waveform_buffer.playback_needs_refresh;

            let (current_position, image_opt) = waveform_buffer.get_image();
            match image_opt {
                Some((image, image_positions)) => {
                    #[cfg(feature = "profiling-audio-draw")]
                    let _before_image = Utc::now();

                    cr.set_source_surface(image, -image_positions.first.x, 0f64);
                    cr.paint();

                    (current_position, image_positions)
                }
                None => {
                    self.clean_cairo_context(cr);

                    #[cfg(feature = "trace-audio-controller")]
                    println!("AudioController::draw no image");

                    return Inhibit(true);
                }
            }
        };

        self.current_position = current_position;

        #[cfg(feature = "profiling-audio-draw")]
        let before_pos = Utc::now();

        let height = f64::from(allocation.height);
        let width = f64::from(allocation.width);
        cr.scale(1f64, 1f64);
        cr.set_source_rgb(1f64, 1f64, 0f64);
        cr.set_font_size(14f64);

        // first position
        let first_text = Timestamp::format(image_positions.first.timestamp, false);
        let first_text_width = 2f64 + cr.text_extents(&first_text).width;
        cr.move_to(2f64, 30f64);
        cr.show_text(&first_text);

        // last position
        if let Some(last_pos) = image_positions.last {
            let last_text = Timestamp::format(last_pos.timestamp, false);
            // align actual width to a 15px multiple box in order to avoid
            // variations due to actual size of individual digits
            let last_text_width = (cr.text_extents(&last_text).width / 15f64).ceil() * 15f64;
            if last_pos.x - last_text_width > first_text_width + 5f64 {
                // last text won't overlap with first text
                cr.move_to(last_pos.x - last_text_width, 30f64);
                cr.show_text(&last_text);
            }

            self.last_visible_pos = last_pos.timestamp;

            // make sure the rest of the image is filled with background's color
            cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
            cr.rectangle(last_pos.x, 0f64, width - last_pos.x, height);
            cr.fill();
        }

        if let Some(current_x) = image_positions.current {
            // draw current pos
            cr.set_source_rgb(1f64, 1f64, 0f64);

            let cursor_text = Timestamp::format(self.current_position, true);
            let cursor_text_width = 5f64 + cr.text_extents(&cursor_text).width;
            let cursor_text_x = if current_x + cursor_text_width < width {
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
        let before_refresh_info = Utc::now();

        match main_ctrl.try_borrow_mut() {
            Ok(mut main_ctrl) => main_ctrl.refresh_info(self.current_position),
            Err(_) => (),
        }

        #[cfg(feature = "profiling-audio-draw")]
        let end = Utc::now();

        #[cfg(feature = "profiling-audio-draw")]
        println!(
            "audio-draw,{},{},{},{},{},{},{}",
            before_init.time().format("%H:%M:%S%.6f"),
            before_lock.time().format("%H:%M:%S%.6f"),
            _before_cndt.time().format("%H:%M:%S%.6f"),
            _before_image.time().format("%H:%M:%S%.6f"),
            before_pos.time().format("%H:%M:%S%.6f"),
            before_refresh_info.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
        );

        Inhibit(true)
    }

    fn refresh(&mut self, main_ctrl: &Rc<RefCell<MainController>>) {
        let allocation = self.drawingarea.get_allocation();
        {
            // refresh the buffer in order to render the waveform
            // in latest conditions
            let requested_duration = self.requested_duration;
            self.dbl_buffer_mtx
                .lock()
                .expect(
                    "AudioController::size-allocate: couldn't lock dbl_buffer_mtx",
                )
                .refresh_with_conditions(
                    Box::new(WaveformConditions::new(
                        requested_duration,
                        allocation.width,
                        allocation.height,
                    )),
                    *main_ctrl.borrow().get_state() == ControllerState::Playing,
                );
        }

        self.redraw();
    }
}
