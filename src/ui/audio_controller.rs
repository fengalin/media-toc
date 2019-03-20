use cairo;
use gdk;
use gdk::{Cursor, CursorType, FrameClockExt, WindowExt};
use glib;
use gtk;
use gtk::prelude::*;
use log::{debug, trace};
use pango;
use pango::{ContextExt, LayoutExt};

use std::{
    boxed::Box,
    cell::RefCell,
    collections::Bound::Included,
    rc::Rc,
    sync::{Arc, Mutex},
};

use crate::{
    media::{DoubleAudioBuffer, PlaybackPipeline, SampleExtractor, QUEUE_SIZE_NS},
    metadata::{MediaInfo, Timestamp},
};

use super::{
    ChaptersBoundaries, DoubleWaveformBuffer, UIController, WaveformBuffer, BACKGROUND_COLOR,
};

const BUFFER_DURATION: u64 = 60_000_000_000; // 60 s
const MIN_REQ_DURATION: f64 = 1_953_125f64; // 2 ms / 1000 px
const MAX_REQ_DURATION: f64 = 32_000_000_000f64; // 32 s / 1000 px
const INIT_REQ_DURATION: f64 = 4_000_000_000f64; // 4 s / 1000 px
const STEP_REQ_DURATION: f64 = 2f64;

const SEEK_STEP_DURATION_DIVISOR: u64 = 2;

// Other UI components refresh period
const OTHER_UI_REFRESH_PERIOD: u64 = 50_000_000; // 50 ms

// Range playback
const MIN_RANGE_DURATION: u64 = 100_000_000; // 100 ms

const HOUR_IN_NANO: u64 = 3_600_000_000_000;

const EXPECTED_FRAME_DURATION: u64 = 16_667;

// Use this text to compute the largest text box for the waveform boundaries
// This is required to position the labels in such a way that they don't
// move constantly depending on the digits width
const BOUNDARY_TEXT_MN: &str = "00:00.000";
const CURSOR_TEXT_MN: &str = "00:00.000.000";
const BOUNDARY_TEXT_H: &str = "00:00:00.000";
const CURSOR_TEXT_H: &str = "00:00:00.000.000";

#[derive(Clone, Debug, PartialEq)]
pub enum AudioControllerState {
    Disabled,
    MovingBoundary(u64),
    Playing,
    Paused,
}

pub enum AudioControllerAction {
    Seek(u64),
    PlayRange { start: u64, end: u64, current: u64 },
}

pub struct AudioController {
    window: gtk::ApplicationWindow,
    container: gtk::Box,
    pub(super) drawingarea: gtk::DrawingArea,
    zoom_in_btn: gtk::ToolButton,
    zoom_out_btn: gtk::ToolButton,
    ref_lbl: gtk::Label,

    font_family: Option<(String)>,
    font_size: f64,
    twice_font_size: f64,
    half_font_size: f64,
    boundary_text_mn_width: f64,
    cursor_text_mn_width: f64,
    boundary_text_h_width: f64,
    cursor_text_h_width: f64,
    pub(super) area_height: f64,
    pub(super) area_width: f64,

    pub(super) state: AudioControllerState,
    playback_needs_refresh: bool,

    requested_duration: f64,
    pub(super) seek_step: u64,
    pub(super) current_position: u64,
    last_other_ui_refresh: u64,
    first_visible_pos: u64,
    last_visible_pos: u64,
    sample_duration: u64,
    sample_step: f64,
    boundaries: Rc<RefCell<ChaptersBoundaries>>,

    waveform_mtx: Arc<Mutex<Box<dyn SampleExtractor>>>,
    pub dbl_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,

    pub(super) update_conditions_async: Option<Rc<Fn()>>,

    pub(super) tick_cb: Option<Rc<Fn(&gtk::DrawingArea, &gdk::FrameClock)>>,
    tick_cb_id: Option<gtk::TickCallbackId>,
}

impl UIController for AudioController {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let is_audio_selected = {
            let info = pipeline.info.read().unwrap();
            info.streams.is_audio_selected()
        };

        if is_audio_selected {
            self.state = AudioControllerState::Paused;

            // Refresh conditions asynchronously so that
            // all widgets are arranged to their target positions
            let update_conditions_fn = Rc::clone(
                self.update_conditions_async
                    .as_ref()
                    .expect("AudioController: no update_conditions_async defined"),
            );
            gtk::idle_add(move || {
                update_conditions_fn();
                glib::Continue(false)
            });
        }
    }

    fn cleanup(&mut self) {
        self.state = AudioControllerState::Disabled;
        self.zoom_in_btn.set_sensitive(false);
        self.zoom_out_btn.set_sensitive(false);
        self.reset_cursor();
        self.playback_needs_refresh = false;
        self.dbl_buffer_mtx.lock().unwrap().cleanup();
        self.requested_duration = INIT_REQ_DURATION;
        self.seek_step = INIT_REQ_DURATION as u64 / SEEK_STEP_DURATION_DIVISOR;
        self.current_position = 0;
        self.last_other_ui_refresh = 0;
        self.first_visible_pos = 0;
        self.last_visible_pos = 0;
        self.sample_duration = 0;
        self.sample_step = 0f64;
        // AudioController accesses self.boundaries as readonly
        // clearing it is under the responsiblity of ChapterTreeManager
        self.update_conditions();
        self.redraw();
    }

    fn streams_changed(&mut self, info: &mut MediaInfo) {
        if info.streams.is_audio_selected() {
            debug!("streams_changed audio selected");
            self.zoom_in_btn.set_sensitive(true);
            self.zoom_out_btn.set_sensitive(true);
            self.container.show();
        } else {
            debug!("streams_changed audio not selected");
            self.container.hide();
        }
    }
}

impl AudioController {
    pub fn new(builder: &gtk::Builder, boundaries: Rc<RefCell<ChaptersBoundaries>>) -> Self {
        let dbl_buffer_mtx = DoubleWaveformBuffer::new_mutex(BUFFER_DURATION);
        let waveform_mtx = dbl_buffer_mtx.lock().unwrap().get_exposed_buffer_mtx();

        AudioController {
            window: builder.get_object("application-window").unwrap(),
            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
            zoom_in_btn: builder.get_object("audio_zoom_in-toolbutton").unwrap(),
            zoom_out_btn: builder.get_object("audio_zoom_out-toolbutton").unwrap(),
            ref_lbl: builder.get_object("title-caption").unwrap(),

            font_family: None,
            font_size: 0f64,
            twice_font_size: 0f64,
            half_font_size: 0f64,
            boundary_text_mn_width: 0f64,
            cursor_text_mn_width: 0f64,
            boundary_text_h_width: 0f64,
            cursor_text_h_width: 0f64,
            area_height: 0f64,
            area_width: 0f64,

            state: AudioControllerState::Disabled,
            playback_needs_refresh: false,

            requested_duration: INIT_REQ_DURATION,
            seek_step: INIT_REQ_DURATION as u64 / SEEK_STEP_DURATION_DIVISOR,

            current_position: 0,
            last_other_ui_refresh: 0,
            first_visible_pos: 0,
            last_visible_pos: 0,
            sample_duration: 0,
            sample_step: 0f64,
            boundaries,

            waveform_mtx,
            dbl_buffer_mtx,

            update_conditions_async: None,

            tick_cb: None,
            tick_cb_id: None,
        }
    }

    pub fn redraw(&self) {
        self.drawingarea.queue_draw();
    }

    pub fn get_seek_back_1st_position(&self, target: u64) -> Option<u64> {
        let (lower_pos, upper_pos, half_window_duration) = {
            let waveform_grd = self.waveform_mtx.lock().unwrap();
            let waveform_buf = waveform_grd
                .as_any()
                .downcast_ref::<WaveformBuffer>()
                .unwrap();
            let limits = waveform_buf.get_limits_as_pos();
            (limits.0, limits.1, waveform_buf.get_half_window_duration())
        };

        // don't step back more than the pipeline queues can handle
        let target_step_back = half_window_duration.min(QUEUE_SIZE_NS);
        if target > target_step_back {
            if target < lower_pos + target_step_back || target > upper_pos {
                Some(target - target_step_back)
            } else {
                // 1st position already available => don't need 2 steps seek back
                None
            }
        } else {
            Some(0)
        }
    }

    pub fn seek(&mut self, position: u64) {
        if self.state == AudioControllerState::Disabled {
            return;
        }

        self.waveform_mtx
            .lock()
            .unwrap()
            .as_mut_any()
            .downcast_mut::<WaveformBuffer>()
            .unwrap()
            .seek(position);

        if self.state != AudioControllerState::Playing {
            // refresh the buffer in order to render the waveform
            // with samples that might not be rendered in current WaveformImage yet
            self.dbl_buffer_mtx.lock().unwrap().refresh();
        }
        self.redraw();
    }

    pub fn switch_to_not_playing(&mut self) {
        if self.state != AudioControllerState::Disabled {
            self.state = AudioControllerState::Paused;
            self.refresh_buffer();
            self.remove_tick_callback();
            self.redraw();
        }
    }

    pub fn switch_to_playing(&mut self) {
        if self.state != AudioControllerState::Disabled {
            self.state = AudioControllerState::Playing;
            self.register_tick_callback();
        }
    }

    pub fn start_play_range(&mut self) {
        if self.state != AudioControllerState::Disabled {
            self.waveform_mtx
                .lock()
                .unwrap()
                .as_mut_any()
                .downcast_mut::<WaveformBuffer>()
                .unwrap()
                .start_play_range();

            self.register_tick_callback();
        }
    }

    pub fn stop_play_range(&mut self) {
        if self.state != AudioControllerState::Disabled {
            self.remove_tick_callback();
        }
    }

    fn remove_tick_callback(&mut self) {
        if let Some(tick_cb_id) = self.tick_cb_id.take() {
            tick_cb_id.remove();
        }
    }

    fn register_tick_callback(&mut self) {
        if self.tick_cb_id.is_some() {
            return;
        }
        let tick_cb = Rc::clone(
            self.tick_cb
                .as_ref()
                .expect("AudioController: no tick callback defined"),
        );
        self.tick_cb_id = Some(self.drawingarea.add_tick_callback(move |da, frame_clock| {
            tick_cb(da, frame_clock);
            glib::Continue(true)
        }));
    }

    pub fn set_cursor(&self, cursor_type: CursorType) {
        let gdk_window = self.window.get_window().unwrap();
        gdk_window.set_cursor(Some(&Cursor::new_for_display(
            &gdk_window.get_display(),
            cursor_type,
        )));
    }

    pub fn reset_cursor(&self) {
        self.window.get_window().unwrap().set_cursor(None);
    }

    fn get_position_at(&self, x: f64) -> Option<u64> {
        if x >= 0f64 && x <= self.area_width {
            let position = self.first_visible_pos
                + (x * self.sample_step).round() as u64 * self.sample_duration;
            if position <= self.last_visible_pos {
                Some(position)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn get_boundary_at(&self, x: f64) -> Option<u64> {
        let position = self.get_position_at(x);
        let position = match position {
            Some(position) => position,
            None => return None,
        };

        let delta = if self.sample_step > 0f64 {
            self.sample_step as u64 * self.sample_duration * 2
        } else {
            1
        };

        let boundaries = self.boundaries.borrow();
        let lower_bound = if position >= delta {
            position - delta
        } else {
            0
        };
        let mut range = boundaries.range((Included(&lower_bound), Included(&(position + delta))));
        range.next().map(|(boundary, _chapters)| *boundary)
    }

    fn adjust_waveform_text_width(&mut self, cr: &cairo::Context) {
        match self.font_family {
            Some(ref family) => {
                cr.select_font_face(family, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                cr.set_font_size(self.font_size);
            }
            None => {
                // Get font specs from the reference label
                let ref_layout = self.ref_lbl.get_layout().unwrap();
                let ref_ctx = ref_layout.get_context().unwrap();
                let font_desc = ref_ctx.get_font_description().unwrap();

                let family = font_desc.get_family().unwrap();
                cr.select_font_face(&family, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                let size = f64::from(ref_layout.get_baseline() / pango::SCALE);
                cr.set_font_size(size);

                self.font_family = Some(family.to_string());
                self.font_size = size;
                self.twice_font_size = 2f64 * size;
                self.half_font_size = 0.5f64 * size;

                self.boundary_text_mn_width = cr.text_extents(BOUNDARY_TEXT_MN).width;
                self.cursor_text_mn_width = cr.text_extents(CURSOR_TEXT_MN).width;
                self.boundary_text_h_width = cr.text_extents(BOUNDARY_TEXT_H).width;
                self.cursor_text_h_width = cr.text_extents(CURSOR_TEXT_H).width;
            }
        }
    }

    pub fn update_conditions(&mut self) {
        if self.state != AudioControllerState::Disabled {
            debug!(
                "update_conditions {}, {}x{}",
                self.requested_duration, self.area_width, self.area_height,
            );

            {
                let waveform_grd = &mut *self.waveform_mtx.lock().unwrap();
                let waveform_buffer = waveform_grd
                    .as_mut_any()
                    .downcast_mut::<WaveformBuffer>()
                    .unwrap();
                waveform_buffer.update_conditions(
                    self.requested_duration,
                    self.area_width as i32,
                    self.area_height as i32,
                );
            }

            self.refresh_buffer();
            self.redraw();
        }
    }

    pub fn zoom_in(&mut self) {
        self.requested_duration /= STEP_REQ_DURATION;
        if self.requested_duration >= MIN_REQ_DURATION {
            self.update_conditions();
        } else {
            self.requested_duration = MIN_REQ_DURATION;
        }
        self.seek_step = self.requested_duration as u64 / SEEK_STEP_DURATION_DIVISOR;
    }

    pub fn zoom_out(&mut self) {
        self.requested_duration *= STEP_REQ_DURATION;
        if self.requested_duration <= MAX_REQ_DURATION {
            self.update_conditions();
        } else {
            self.requested_duration = MAX_REQ_DURATION;
        }
        self.seek_step = self.requested_duration as u64 / SEEK_STEP_DURATION_DIVISOR;
    }

    pub fn refresh_buffer(&self) {
        self.dbl_buffer_mtx.lock().unwrap().refresh();
    }

    pub fn tick(&mut self) {
        if self.playback_needs_refresh {
            trace!("tick forcing refresh");
            self.refresh_buffer();
        }

        self.redraw();
    }

    pub fn draw(&mut self, da: &gtk::DrawingArea, cr: &cairo::Context) -> Option<u64> {
        cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
        cr.paint();

        if self.state == AudioControllerState::Disabled {
            // Not active yet, don't display
            debug!("draw still disabled, not drawing");
            return None;
        }

        // Get frame timings
        let (last_frame_time, next_frame_time) = {
            let frame_clock = da.get_frame_clock().unwrap();
            match frame_clock.get_current_timings() {
                Some(frame_timings) => {
                    let frame_time = frame_timings.get_frame_time() as u64;
                    match frame_timings.get_predicted_presentation_time() {
                        Some(predicted_presentation_time) => {
                            (frame_time, predicted_presentation_time.get())
                        }
                        None => {
                            // predicted_presentation_time not available => estimate it
                            (frame_time, frame_time + EXPECTED_FRAME_DURATION)
                        }
                    }
                }
                None => {
                    debug!("can't get frame timings");
                    return None;
                }
            }
        };

        // Get waveform and positions
        let (current_position, image_positions) = {
            let waveform_grd = &mut *self.waveform_mtx.lock().unwrap();
            let waveform_buffer = waveform_grd
                .as_mut_any()
                .downcast_mut::<WaveformBuffer>()
                .unwrap();

            self.playback_needs_refresh = waveform_buffer.playback_needs_refresh;

            let (current_position, image_opt) =
                waveform_buffer.get_image(last_frame_time, next_frame_time);
            match image_opt {
                Some((image, image_positions)) => {
                    image.with_surface_external_context(cr, |cr, surface| {
                        cr.set_source_surface(surface, -image_positions.first.x, 0f64);
                        cr.paint();
                    });

                    (current_position, image_positions)
                }
                None => {
                    debug!("draw no image");
                    return None;
                }
            }
        };

        self.current_position = current_position;
        self.first_visible_pos = image_positions.first.timestamp;
        self.sample_duration = image_positions.sample_duration;
        self.sample_step = image_positions.sample_step;

        cr.scale(1f64, 1f64);
        cr.set_source_rgb(1f64, 1f64, 0f64);
        self.adjust_waveform_text_width(cr);

        // first position
        let first_text = Timestamp::format(self.first_visible_pos, false);
        let first_text_end = if self.first_visible_pos < HOUR_IN_NANO {
            2f64 + self.boundary_text_mn_width
        } else {
            2f64 + self.boundary_text_h_width
        };
        cr.move_to(2f64, self.twice_font_size);
        cr.show_text(&first_text);

        // last position
        if let Some(last_pos) = image_positions.last {
            let last_text = Timestamp::format(last_pos.timestamp, false);
            let last_text_start = if last_pos.timestamp < HOUR_IN_NANO {
                2f64 + self.boundary_text_mn_width
            } else {
                2f64 + self.boundary_text_h_width
            };
            if last_pos.x - last_text_start > first_text_end + 5f64 {
                // last text won't overlap with first text
                cr.move_to(last_pos.x - last_text_start, self.twice_font_size);
                cr.show_text(&last_text);
            }

            self.last_visible_pos = last_pos.timestamp;

            // Draw in-range chapters boundaries
            let boundaries = self.boundaries.borrow();

            let chapter_range = boundaries.range((
                Included(&self.first_visible_pos),
                Included(&last_pos.timestamp),
            ));

            cr.set_source_rgb(0.5f64, 0.6f64, 1f64);
            cr.set_line_width(1f64);
            let boundary_y0 = self.twice_font_size + 5f64;
            let text_base = self.area_height - self.half_font_size;

            for (boundary, chapters) in chapter_range {
                if *boundary >= self.first_visible_pos {
                    let x = ((boundary - self.first_visible_pos) / image_positions.sample_duration)
                        as f64
                        / image_positions.sample_step;
                    cr.move_to(x, boundary_y0);
                    cr.line_to(x, self.area_height);
                    cr.stroke();

                    if let Some(ref prev_chapter) = chapters.prev {
                        cr.move_to(
                            x - 5f64 - cr.text_extents(&prev_chapter.title).width,
                            text_base,
                        );
                        cr.show_text(&prev_chapter.title);
                    }

                    if let Some(ref next_chapter) = chapters.next {
                        cr.move_to(x + 5f64, text_base);
                        cr.show_text(&next_chapter.title);
                    }
                }
            }
        }

        if let Some(current_x) = image_positions.current {
            // draw current pos
            cr.set_source_rgb(1f64, 1f64, 0f64);

            let cursor_text = Timestamp::format(self.current_position, true);
            let cursor_text_end = if self.current_position < HOUR_IN_NANO {
                5f64 + self.cursor_text_mn_width
            } else {
                5f64 + self.cursor_text_h_width
            };
            let cursor_text_x = if current_x + cursor_text_end < self.area_width {
                current_x + 5f64
            } else {
                current_x - cursor_text_end
            };
            cr.move_to(cursor_text_x, self.font_size);
            cr.show_text(&cursor_text);

            cr.set_line_width(1f64);
            cr.move_to(current_x, 0f64);
            cr.line_to(current_x, self.area_height - self.twice_font_size);
            cr.stroke();
        }

        // update other UI position
        // Note: we go through the audio controller here in order
        // to reduce position queries on the ref gst element
        // TODO: see if a local clock reduces percetibles short hangs
        // while the waveform scrolls
        let must_refresh_other_ui = {
            match self.state {
                AudioControllerState::Playing => {
                    if self.current_position >= self.last_other_ui_refresh {
                        self.current_position > self.last_other_ui_refresh + OTHER_UI_REFRESH_PERIOD
                    } else {
                        true
                    }
                }
                AudioControllerState::Paused => true,
                AudioControllerState::MovingBoundary(_boundary) => true,
                _ => false,
            }
        };

        if must_refresh_other_ui {
            self.last_other_ui_refresh = self.current_position;
            Some(current_position)
        } else {
            None
        }
    }

    pub fn motion_notify(&mut self, event_motion: &gdk::EventMotion) -> Option<(u64, u64)> {
        let (x, _y) = event_motion.get_position();
        match self.state {
            AudioControllerState::Paused => {
                match self.get_boundary_at(x) {
                    Some(_boundary) => self.set_cursor(CursorType::SbHDoubleArrow),
                    None => self.reset_cursor(),
                };
                None
            }
            AudioControllerState::Playing => None,
            AudioControllerState::MovingBoundary(boundary) => match self.get_position_at(x) {
                Some(position) => Some((boundary, position)),
                None => None,
            },
            _ => None,
        }
    }

    pub fn button_press(
        &mut self,
        event_button: &gdk::EventButton,
    ) -> Option<AudioControllerAction> {
        match event_button.get_button() {
            1 => {
                // left button
                let position = self.get_position_at(event_button.get_position().0)?;
                let must_seek = match self.state {
                    AudioControllerState::Paused => self
                        .get_boundary_at(event_button.get_position().0)
                        .map_or(true, |boundary| {
                            self.state = AudioControllerState::MovingBoundary(boundary);
                            false
                        }),
                    _ => true,
                };

                if must_seek {
                    Some(AudioControllerAction::Seek(position))
                } else {
                    None
                }
            }
            3 => {
                // right button => segment playback
                self.get_position_at(event_button.get_position().0)
                    .map(|start| AudioControllerAction::PlayRange {
                        start,
                        end: start + MIN_RANGE_DURATION.max(self.last_visible_pos - start),
                        current: self.current_position,
                    })
            }
            _ => None,
        }
    }
}
