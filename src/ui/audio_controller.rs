use cairo;
use gdk;
use gdk::FrameClockExt;
use glib;
use gstreamer as gst;
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

use media::{DoubleAudioBuffer, Duration, PlaybackPipeline, Timestamp, QUEUE_SIZE};
use metadata::MediaInfo;
use renderers::{DoubleWaveformRenderer, WaveformMetrics, WaveformRenderer};

use super::{ChaptersBoundaries, UIController, UIEventSender};

const BUFFER_DURATION: Duration = Duration::from_secs(60);
const INIT_REQ_DURATION_FOR_1000PX: Duration = Duration::from_secs(4);
const MIN_REQ_DURATION_FOR_1000PX: Duration = Duration::from_nanos(1_953_125); // 4s / 2^11
const MAX_REQ_DURATION_FOR_1000PX: Duration = Duration::from_secs(32);
const REQ_DURATION_SCALE_FACTOR: u64 = 2;

const SEEK_STEP_DURATION_DIVISOR: u64 = 2;

// Other UI components refresh period
const OTHER_UI_REFRESH_PERIOD: Duration = Duration::from_millis(50);

// Range playback
const MIN_RANGE_DURATION: Duration = Duration::from_millis(100);

const EXPECTED_FRAME_DURATION: Duration = Duration::from_frequency(60);

const ONE_HOUR: Duration = Duration::from_secs(60 * 60);

const BACKGROUND_COLOR: (f64, f64, f64) = (0.2f64, 0.2235f64, 0.2314f64);
const CURSOR_COLOR: (f64, f64, f64) = (1f64, 1f64, 0f64);

// Use this text to compute the largest text box for the waveform limits
// This is required to position the labels in such a way they don't
// move constantly depending on the digits width
const LIMIT_TEXT_MN: &str = "00:00.000";
const LIMIT_TEXT_H: &str = "00:00:00.000";
const CURSOR_TEXT_MN: &str = "00:00.000.000";
const CURSOR_TEXT_H: &str = "00:00:00.000.000";

#[derive(Debug, PartialEq)]
pub enum ControllerState {
    Disabled,
    CursorAboveBoundary(Timestamp),
    MovingBoundary(Timestamp),
    Playing,
    Paused,
}

#[derive(Default)]
struct TextMetrics {
    font_family: Option<(String)>,
    font_size: f64,
    twice_font_size: f64,
    half_font_size: f64,
    limit_mn_width: f64,
    limit_h_width: f64,
    limit_y: f64,
    cursor_mn_width: f64,
    cursor_h_width: f64,
    cursor_y: f64,
}

pub struct AudioController {
    ui_event: UIEventSender,

    container: gtk::Box,
    pub(super) drawingarea: gtk::DrawingArea,
    zoom_in_btn: gtk::ToolButton,
    zoom_out_btn: gtk::ToolButton,
    ref_lbl: gtk::Label,

    pub(super) area_height: f64,
    pub(super) area_width: f64,

    pub(super) state: ControllerState,
    playback_needs_refresh: bool,

    requested_duration: Duration,
    pub(super) seek_step: Duration,

    text_metrics: TextMetrics,
    waveform_metrics: WaveformMetrics,

    last_other_ui_refresh: Timestamp,
    boundaries: Rc<RefCell<ChaptersBoundaries>>,

    waveform_mtx: Arc<Mutex<Box<WaveformRenderer>>>,
    pub dbl_buffer_mtx: Arc<Mutex<DoubleAudioBuffer<WaveformRenderer>>>,

    pub(super) update_conditions_async: Option<Box<Fn() -> glib::SourceId>>,

    pub(super) tick_cb: Option<Rc<Fn(&gtk::DrawingArea, &gdk::FrameClock)>>,
    tick_cb_id: Option<gtk::TickCallbackId>,
}

impl UIController for AudioController {
    fn new_media(&mut self, pipeline: &PlaybackPipeline<WaveformRenderer>) {
        let is_audio_selected = {
            let info = pipeline.info.read().unwrap();
            info.streams.is_audio_selected()
        };

        if is_audio_selected {
            self.state = ControllerState::Paused;

            // Refresh conditions asynchronously so that
            // all widgets are arranged to their target positions
            self.update_conditions_async.as_ref().unwrap()();
        }
    }

    fn cleanup(&mut self) {
        self.state = ControllerState::Disabled;
        self.zoom_in_btn.set_sensitive(false);
        self.zoom_out_btn.set_sensitive(false);
        self.playback_needs_refresh = false;
        self.dbl_buffer_mtx.lock().unwrap().cleanup();
        self.requested_duration = INIT_REQ_DURATION_FOR_1000PX;
        self.seek_step = INIT_REQ_DURATION_FOR_1000PX / SEEK_STEP_DURATION_DIVISOR;
        self.last_other_ui_refresh = Timestamp::default();
        // AudioController accesses self.boundaries as readonly
        // clearing it is under the responsiblity of ChapterTreeManager
        self.text_metrics = TextMetrics::default();
        self.update_conditions();
        self.redraw();
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
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
    pub fn new(
        builder: &gtk::Builder,
        ui_event_sender: UIEventSender,
        boundaries: Rc<RefCell<ChaptersBoundaries>>,
    ) -> Self {
        let dbl_buffer_mtx = DoubleWaveformRenderer::new_mutex(BUFFER_DURATION);
        let waveform_mtx = dbl_buffer_mtx.lock().unwrap().get_exposed_buffer_mtx();

        AudioController {
            ui_event: ui_event_sender,

            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
            zoom_in_btn: builder.get_object("audio_zoom_in-toolbutton").unwrap(),
            zoom_out_btn: builder.get_object("audio_zoom_out-toolbutton").unwrap(),
            ref_lbl: builder.get_object("title-caption").unwrap(),

            area_height: 0f64,
            area_width: 0f64,

            state: ControllerState::Disabled,
            playback_needs_refresh: false,

            requested_duration: INIT_REQ_DURATION_FOR_1000PX,
            seek_step: INIT_REQ_DURATION_FOR_1000PX / SEEK_STEP_DURATION_DIVISOR,

            text_metrics: TextMetrics::default(),
            waveform_metrics: WaveformMetrics::default(),

            last_other_ui_refresh: Timestamp::default(),
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

    pub fn get_seek_back_1st_ts(&self, target: Timestamp) -> Option<Timestamp> {
        let (lower_ts, upper_ts, half_window_duration) = {
            let waveform_renderer = self.waveform_mtx.lock().unwrap();
            let limits = waveform_renderer.get_limits_as_ts();
            (
                limits.0,
                limits.1,
                waveform_renderer.get_half_window_duration(),
            )
        };

        // don't step back more than the pipeline queues can handle
        let step_back_duration = half_window_duration.min(QUEUE_SIZE);
        if target > step_back_duration {
            if target < lower_ts + step_back_duration || target > upper_ts {
                Some(target - step_back_duration)
            } else {
                // 1st timestamp already available => don't need 2 steps seek back
                None
            }
        } else {
            Some(Timestamp::default())
        }
    }

    pub fn seek(&mut self, target: Timestamp) {
        if self.state == ControllerState::Disabled {
            return;
        }

        self.waveform_mtx.lock().unwrap().seek(target);
        if self.state != ControllerState::Playing {
            // refresh the buffer in order to render the waveform
            // with samples that might not be rendered in current WaveformImage yet
            self.dbl_buffer_mtx.lock().unwrap().refresh();
        }
        self.redraw();
    }

    pub fn switch_to_not_playing(&mut self) {
        if self.state != ControllerState::Disabled {
            self.state = ControllerState::Paused;
            self.refresh_buffer();
            self.remove_tick_callback();
            self.redraw();
        }
    }

    pub fn switch_to_playing(&mut self) {
        if self.state != ControllerState::Disabled {
            self.state = ControllerState::Playing;
            self.register_tick_callback();
        }
    }

    pub fn start_play_range(&mut self) {
        if self.state != ControllerState::Disabled {
            self.waveform_mtx.lock().unwrap().start_play_range();
            self.register_tick_callback();
        }
    }

    pub fn stop_play_range(&mut self) {
        if self.state != ControllerState::Disabled {
            self.remove_tick_callback();
        }
    }

    pub fn seek_complete(&mut self) {
        self.waveform_mtx.lock().unwrap().seek_complete();
        self.last_other_ui_refresh = Timestamp::default();
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

    fn get_ts_at(&self, x: f64) -> Option<Timestamp> {
        if x >= 0f64 && x <= self.area_width {
            let ts = self.waveform_metrics.first_ts
                + self.waveform_metrics.sample_duration
                    * ((x * self.waveform_metrics.sample_step).round() as u64);
            if ts <= self.waveform_metrics.last.ts {
                Some(ts)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn get_boundary_at(&self, x: f64) -> Option<Timestamp> {
        let ts = match self.get_ts_at(x) {
            Some(ts) => ts,
            None => return None,
        };

        let tolerance = if self.waveform_metrics.sample_step > 1f64 {
            self.waveform_metrics.sample_duration * 2 * (self.waveform_metrics.sample_step as u64)
        } else {
            Duration::from_nanos(1)
        };

        let boundaries = self.boundaries.borrow();
        let lower_bound = if ts >= tolerance {
            ts - tolerance
        } else {
            Timestamp::default()
        };
        let mut range = boundaries.range((Included(&lower_bound), Included(&(ts + tolerance))));
        range.next().map(|(boundary, _chapters)| *boundary)
    }

    #[inline]
    fn adjust_waveform_text_width(&mut self, cr: &cairo::Context) {
        match self.text_metrics.font_family {
            Some(ref family) => {
                cr.select_font_face(family, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                cr.set_font_size(self.text_metrics.font_size);
            }
            None => {
                // Get font specs from the reference label
                let ref_layout = self.ref_lbl.get_layout().unwrap();
                let ref_ctx = ref_layout.get_context().unwrap();
                let font_desc = ref_ctx.get_font_description().unwrap();

                let family = font_desc.get_family().unwrap();
                cr.select_font_face(&family, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                let font_size = f64::from(ref_layout.get_baseline() / pango::SCALE);
                cr.set_font_size(font_size);

                self.text_metrics.font_family = Some(family.to_string());
                self.text_metrics.font_size = font_size;
                self.text_metrics.twice_font_size = 2f64 * font_size;
                self.text_metrics.half_font_size = 0.5f64 * font_size;

                self.text_metrics.limit_mn_width = cr.text_extents(LIMIT_TEXT_MN).width;
                self.text_metrics.limit_h_width = cr.text_extents(LIMIT_TEXT_H).width;
                self.text_metrics.limit_y = 2f64 * font_size;
                self.text_metrics.cursor_mn_width = cr.text_extents(CURSOR_TEXT_MN).width;
                self.text_metrics.cursor_h_width = cr.text_extents(CURSOR_TEXT_H).width;
                self.text_metrics.cursor_y = font_size;
            }
        }
    }

    pub fn update_conditions(&mut self) {
        self.waveform_metrics = WaveformMetrics::default();

        if self.state != ControllerState::Disabled {
            debug!(
                "update_conditions {}, {}x{}",
                self.requested_duration, self.area_width, self.area_height,
            );

            {
                self.waveform_mtx.lock().unwrap().update_conditions(
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
        self.requested_duration /= REQ_DURATION_SCALE_FACTOR;
        if self.requested_duration >= MIN_REQ_DURATION_FOR_1000PX {
            self.update_conditions();
        } else {
            self.requested_duration = MIN_REQ_DURATION_FOR_1000PX;
        }
        self.seek_step = self.requested_duration / SEEK_STEP_DURATION_DIVISOR;
    }

    pub fn zoom_out(&mut self) {
        self.requested_duration *= REQ_DURATION_SCALE_FACTOR;
        if self.requested_duration <= MAX_REQ_DURATION_FOR_1000PX {
            self.update_conditions();
        } else {
            self.requested_duration = MAX_REQ_DURATION_FOR_1000PX;
        }
        self.seek_step = self.requested_duration / SEEK_STEP_DURATION_DIVISOR;
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

    pub fn draw(&mut self, da: &gtk::DrawingArea, cr: &cairo::Context) -> Option<Timestamp> {
        cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
        cr.paint();

        if self.state == ControllerState::Disabled {
            // Not active yet, don't display
            debug!("draw still disabled, not drawing");
            return None;
        }

        // Get frame timings
        let (last_frame_ts, next_frame_ts) =
            da.get_frame_clock()?
                .get_current_timings()
                .map(|frame_timings| {
                    let frame_time = Timestamp::new(1_000 * frame_timings.get_frame_time() as u64);
                    frame_timings.get_predicted_presentation_time().map_or_else(
                        || (frame_time, frame_time + EXPECTED_FRAME_DURATION),
                        |predicted_pres_time| {
                            (
                                frame_time,
                                Timestamp::new(1_000 * predicted_pres_time.get() as u64),
                            )
                        },
                    )
                })?;

        // Draw the waveform
        self.waveform_metrics = {
            let waveform_renderer = &mut *self.waveform_mtx.lock().unwrap();
            self.playback_needs_refresh = waveform_renderer.playback_needs_refresh;

            waveform_renderer.update_first_visible_sample(last_frame_ts, next_frame_ts);
            waveform_renderer.render(cr)?
        };

        self.adjust_waveform_text_width(cr);

        // Draw in-range chapters boundaries
        let boundaries = self.boundaries.borrow();

        let chapter_range = boundaries.range((
            Included(&self.waveform_metrics.first_ts),
            Included(&self.waveform_metrics.last.ts),
        ));

        cr.set_source_rgb(0.5f64, 0.6f64, 1f64);
        cr.set_line_width(1f64);

        let boundary_y0 = self.text_metrics.twice_font_size + 5f64;
        let text_base = self.area_height - self.text_metrics.half_font_size;

        for (boundary, chapters) in chapter_range {
            if *boundary >= self.waveform_metrics.first_ts {
                let x = ((*boundary - self.waveform_metrics.first_ts)
                    .get_index_range(self.waveform_metrics.sample_duration))
                .as_f64()
                    / self.waveform_metrics.sample_step;
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

        cr.set_source_rgb(1f64, 1f64, 0f64);

        // first position
        let first_text = self
            .waveform_metrics
            .first_ts
            .get_4_humans()
            .as_string(false);
        let first_text_end = if self.waveform_metrics.first_ts < ONE_HOUR {
            2f64 + self.text_metrics.limit_mn_width
        } else {
            2f64 + self.text_metrics.limit_h_width
        };
        cr.move_to(2f64, self.text_metrics.limit_y);
        cr.show_text(&first_text);

        // last position
        let last_text = self
            .waveform_metrics
            .last
            .ts
            .get_4_humans()
            .as_string(false);
        let last_text_start = if self.waveform_metrics.last.ts < ONE_HOUR {
            2f64 + self.text_metrics.limit_mn_width
        } else {
            2f64 + self.text_metrics.limit_h_width
        };
        if self.waveform_metrics.last.x - last_text_start > first_text_end + 5f64 {
            // last text won't overlap with first text
            cr.move_to(
                self.waveform_metrics.last.x - last_text_start,
                self.text_metrics.limit_y,
            );
            cr.show_text(&last_text);
        }

        if let Some(cursor) = &self.waveform_metrics.cursor {
            // draw cursor
            cr.set_source_rgb(CURSOR_COLOR.0, CURSOR_COLOR.1, CURSOR_COLOR.2);

            let cursor_text = cursor.ts.get_4_humans().as_string(true);
            let cursor_text_end = if cursor.ts < ONE_HOUR {
                5f64 + self.text_metrics.cursor_mn_width
            } else {
                5f64 + self.text_metrics.cursor_h_width
            };
            let cursor_text_x = if cursor.x + cursor_text_end < self.area_height {
                cursor.x + 5f64
            } else {
                cursor.x - cursor_text_end
            };
            cr.move_to(cursor_text_x, self.text_metrics.cursor_y);
            cr.show_text(&cursor_text);

            cr.set_line_width(1f64);
            cr.move_to(cursor.x, 0f64);
            cr.line_to(
                cursor.x,
                self.area_height - 2f64 * self.text_metrics.cursor_y,
            );
            cr.stroke();
        }

        // update other UI position
        // Note: we go through the audio controller here in order
        // to reduce position queries on the ref gst element
        let cursor_ts = self
            .waveform_metrics
            .cursor
            .as_ref()
            .map(|cursor| cursor.ts)?;
        match self.state {
            ControllerState::Playing => {
                if cursor_ts >= self.last_other_ui_refresh
                    && cursor_ts <= self.last_other_ui_refresh + OTHER_UI_REFRESH_PERIOD
                {
                    return None;
                }
            }
            ControllerState::Paused => (),
            ControllerState::MovingBoundary(_) => (),
            _ => return None,
        }

        self.last_other_ui_refresh = cursor_ts;

        Some(cursor_ts)
    }

    pub fn motion_notify(
        &mut self,
        event_motion: &gdk::EventMotion,
    ) -> Option<(Timestamp, Timestamp)> {
        let (x, _y) = event_motion.get_position();

        match self.state {
            ControllerState::Playing => (),
            ControllerState::MovingBoundary(boundary) => {
                return self.get_ts_at(x).map(|position| (boundary, position));
            }
            ControllerState::Paused => {
                if let Some(boundary) = self.get_boundary_at(x) {
                    self.state = ControllerState::CursorAboveBoundary(boundary);
                    self.ui_event.set_cursor_double_arrow();
                }
            }
            ControllerState::CursorAboveBoundary(_) => {
                if let Some(boundary) = self.get_boundary_at(x) {
                    self.state = ControllerState::CursorAboveBoundary(boundary);
                } else {
                    self.state = ControllerState::Paused;
                    self.ui_event.reset_cursor();
                }
            }
            _ => (),
        }

        None
    }

    pub fn leave_drawing_area(&mut self) {
        match self.state {
            ControllerState::Playing => (),
            ControllerState::Paused => (),
            ControllerState::MovingBoundary(_) | ControllerState::CursorAboveBoundary(_) => {
                self.state = ControllerState::Paused;
                self.ui_event.reset_cursor()
            }
            _ => (),
        }
    }

    pub fn button_pressed(&mut self, event_button: &gdk::EventButton) {
        match event_button.get_button() {
            1 => {
                // left button
                if let Some(ts) = self.get_ts_at(event_button.get_position().0) {
                    match self.state {
                        ControllerState::Playing | ControllerState::Paused => {
                            self.ui_event.seek(ts, gst::SeekFlags::ACCURATE);
                        }
                        ControllerState::CursorAboveBoundary(boundary) => {
                            self.state = ControllerState::MovingBoundary(boundary);
                        }
                        _ => (),
                    }
                }
            }
            3 => {
                // right button => segment playback in Paused state
                if self.state == ControllerState::Paused {
                    if let Some(cursor) = &self.waveform_metrics.cursor {
                        let cursor_ts = cursor.ts;
                        if let Some(start) = self.get_ts_at(event_button.get_position().0) {
                            let end = start
                                + MIN_RANGE_DURATION.max(self.waveform_metrics.last.ts - start);
                            self.ui_event.play_range(start, end, cursor_ts);
                        }
                    }
                }
            }
            _ => (),
        }
    }

    pub fn button_released(&mut self, event_button: &gdk::EventButton) {
        if let ControllerState::MovingBoundary(boundary) = self.state {
            if 1 == event_button.get_button() {
                // left button
                self.state = ControllerState::CursorAboveBoundary(boundary);
            }
        }
    }
}
