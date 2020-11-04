use glib::clone;
use gtk::prelude::*;
use log::{debug, trace};

use std::{
    boxed::Box,
    cell::RefCell,
    collections::Bound::Included,
    rc::Rc,
    sync::{Arc, Mutex},
};

use media::{DoubleAudioBuffer, SampleIndexRange, Timestamp, QUEUE_SIZE};
use metadata::{Duration, MediaInfo};
use renderers::{DoubleWaveformRenderer, ImagePositions, WaveformRenderer, BACKGROUND_COLOR};

use super::{ChaptersBoundaries, PlaybackPipeline, UIController, UIEventSender};

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

const ONE_HOUR: Duration = Duration::from_secs(60 * 60);

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
    // FIXME those two are more cursor specific than Controller specific
    CursorAboveBoundary(Timestamp),
    MovingBoundary(Timestamp),
    Playing,
    PlayingRange(Timestamp),
    Paused,
    PausedPlayingRange(Timestamp),
    SeekingPaused,
    SeekingPlaying,
}

#[derive(Default)]
struct TextMetrics {
    font_family: Option<String>,
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
    pub(super) zoom_in_action: gio::SimpleAction,
    zoom_out_btn: gtk::ToolButton,
    pub(super) zoom_out_action: gio::SimpleAction,
    ref_lbl: gtk::Label,

    pub(super) step_forward_action: gio::SimpleAction,
    pub(super) step_back_action: gio::SimpleAction,

    text_metrics: TextMetrics,

    pub(super) area_height: f64,
    pub(super) area_width: f64,
    pub(super) pending_update_conditions: bool,

    pub(super) state: ControllerState,
    playback_needs_refresh: bool,

    requested_duration: Duration,
    pub(super) seek_step: Duration,
    last_other_ui_refresh: Timestamp,
    positions: ImagePositions,
    boundaries: Rc<RefCell<ChaptersBoundaries>>,

    waveform_renderer_mtx: Arc<Mutex<Box<WaveformRenderer>>>,
    pub dbl_renderer_mtx: Arc<Mutex<DoubleAudioBuffer<WaveformRenderer>>>,

    tick_cb_id: Option<gtk::TickCallbackId>,
}

impl UIController for AudioController {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let is_audio_selected = {
            let info = pipeline.info.read().unwrap();
            info.streams.is_audio_selected()
        };

        if is_audio_selected {
            self.state = ControllerState::Paused;

            // Refresh conditions asynchronously so that
            // all widgets are arranged to their target positions
            self.update_conditions_async();
        }

        // FIXME: step forward / back actions should probably be
        // defined in the InfoController. On the other hand, they
        // depend on the seek_step which is defined in AudioController
        // since it depends on the zoom factor, which has to do with
        // the waveform.
        self.step_forward_action.set_enabled(true);
        self.step_back_action.set_enabled(true);
    }

    fn cleanup(&mut self) {
        self.state = ControllerState::Disabled;
        self.zoom_in_btn.set_sensitive(false);
        self.zoom_in_action.set_enabled(false);
        self.zoom_out_btn.set_sensitive(false);
        self.zoom_out_action.set_enabled(false);
        self.step_forward_action.set_enabled(false);
        self.step_back_action.set_enabled(false);
        self.playback_needs_refresh = false;
        self.dbl_renderer_mtx.lock().unwrap().cleanup();
        self.requested_duration = INIT_REQ_DURATION_FOR_1000PX;
        self.seek_step = INIT_REQ_DURATION_FOR_1000PX / SEEK_STEP_DURATION_DIVISOR;
        self.last_other_ui_refresh = Timestamp::default();
        self.positions = ImagePositions::default();
        // AudioController accesses self.boundaries as readonly
        // clearing it is under the responsiblity of ChapterTreeManager
        self.text_metrics = TextMetrics::default();
        self.update_conditions(None);
        self.refresh();
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        if info.streams.is_audio_selected() {
            debug!("streams_changed audio selected");
            self.zoom_in_btn.set_sensitive(true);
            self.zoom_in_action.set_enabled(true);
            self.zoom_out_btn.set_sensitive(true);
            self.zoom_out_action.set_enabled(true);
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
        ui_event: UIEventSender,
        boundaries: Rc<RefCell<ChaptersBoundaries>>,
    ) -> Self {
        let dbl_waveform = DoubleWaveformRenderer::new_dbl_audio_buffer(BUFFER_DURATION);
        let waveform_renderer_mtx = dbl_waveform.exposed_buffer_mtx();

        let mut ctrl = AudioController {
            ui_event,

            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
            zoom_in_btn: builder.get_object("audio_zoom_in-toolbutton").unwrap(),
            zoom_in_action: gio::SimpleAction::new("zoom_in", None),
            zoom_out_btn: builder.get_object("audio_zoom_out-toolbutton").unwrap(),
            zoom_out_action: gio::SimpleAction::new("zoom_out", None),
            ref_lbl: builder.get_object("title-caption").unwrap(),

            step_forward_action: gio::SimpleAction::new("step_forward", None),
            step_back_action: gio::SimpleAction::new("step_back", None),

            text_metrics: TextMetrics::default(),

            area_height: 0f64,
            area_width: 0f64,
            pending_update_conditions: false,

            state: ControllerState::Disabled,
            playback_needs_refresh: false,

            requested_duration: INIT_REQ_DURATION_FOR_1000PX,
            seek_step: INIT_REQ_DURATION_FOR_1000PX / SEEK_STEP_DURATION_DIVISOR,

            last_other_ui_refresh: Timestamp::default(),
            positions: ImagePositions::default(),
            boundaries,

            waveform_renderer_mtx,
            dbl_renderer_mtx: Arc::new(Mutex::new(dbl_waveform)),

            tick_cb_id: None,
        };

        ctrl.cleanup();

        ctrl
    }

    pub fn redraw(&self) {
        self.drawingarea.queue_draw();
    }

    pub fn refresh(&mut self) {
        self.refresh_buffer();
        self.waveform_renderer_mtx.lock().unwrap().refresh();

        self.last_other_ui_refresh = Timestamp::default();
        self.drawingarea.queue_draw();
    }

    /// Finds the first timestamp for a seek in Paused state.
    ///
    /// This is used as an attempt to center the waveform on the target timestamp.
    pub fn first_ts_for_paused_seek(&self, target: Timestamp) -> Option<Timestamp> {
        let (lower_ts, upper_ts, half_window_duration) = {
            let waveform_renderer = self.waveform_renderer_mtx.lock().unwrap();
            let limits = waveform_renderer.limits_as_ts();
            (limits.0, limits.1, waveform_renderer.half_window_duration())
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

    pub fn pause(&mut self) {
        match self.state {
            ControllerState::Playing => {
                self.state = ControllerState::Paused;
            }
            ControllerState::PlayingRange(ts) => {
                self.state = ControllerState::PausedPlayingRange(ts);
            }
            _ => return,
        }

        self.waveform_renderer_mtx.lock().unwrap().freeze();

        self.refresh_buffer();
        self.remove_tick_callback();
        self.redraw();
    }

    pub fn play(&mut self) {
        match self.state {
            ControllerState::Paused => {
                self.state = ControllerState::Playing;
            }
            ControllerState::PausedPlayingRange(ts) => {
                self.state = ControllerState::PlayingRange(ts);
            }
            _ => return,
        }

        self.waveform_renderer_mtx.lock().unwrap().release();
        self.register_tick_callback();
    }

    pub fn seek(&mut self) {
        match self.state {
            ControllerState::Playing => {
                self.state = ControllerState::SeekingPlaying;
            }
            ControllerState::Paused => self.state = ControllerState::SeekingPaused,
            ControllerState::PlayingRange(_) => {
                self.remove_tick_callback();
                self.state = ControllerState::SeekingPaused;
            }
            ControllerState::PausedPlayingRange(_) => {
                self.state = ControllerState::SeekingPaused;
            }
            _ => (),
        }
    }

    pub fn seek_done(&mut self, ts: Timestamp) {
        match self.state {
            ControllerState::SeekingPlaying => {
                self.state = ControllerState::Playing;
                self.waveform_renderer_mtx.lock().unwrap().seek_done(ts);
                self.last_other_ui_refresh = Timestamp::default();
            }
            ControllerState::SeekingPaused => {
                self.state = ControllerState::Paused;
                self.waveform_renderer_mtx.lock().unwrap().seek_done(ts);
                self.refresh();
            }
            _ => unreachable!("seek_done in {:?}", self.state),
        }
    }

    pub fn start_play_range(&mut self, to_restore: Timestamp) {
        match self.state {
            ControllerState::PlayingRange(_) => {
                self.state = ControllerState::PlayingRange(to_restore);
            }
            ControllerState::Paused | ControllerState::PausedPlayingRange(_) => {
                self.state = ControllerState::PlayingRange(to_restore);
                self.register_tick_callback();
            }
            _ => unreachable!("start_play_range in {:?}", self.state),
        }
    }

    pub fn stop_play_range(&mut self) {
        match self.state {
            ControllerState::PlayingRange(_) => {
                self.remove_tick_callback();
                self.state = ControllerState::Paused;
            }
            _ => unreachable!("stop_play_range in {:?}", self.state),
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
        self.tick_cb_id = Some(self.drawingarea.add_tick_callback(
            clone!(@strong self.ui_event as ui_event => move |_, _| {
                ui_event.tick();
                glib::Continue(true)
            }),
        ));
    }

    fn ts_at(&self, x: f64) -> Option<Timestamp> {
        if x >= 0f64 && x <= self.area_width {
            let ts = self.positions.offset.ts
                + self.positions.sample_duration
                    * ((x * self.positions.sample_step).round() as u64);
            if ts <= self.positions.last.ts {
                Some(ts)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn boundary_at(&self, x: f64) -> Option<Timestamp> {
        let ts = match self.ts_at(x) {
            Some(ts) => ts,
            None => return None,
        };

        let tolerance = if self.positions.sample_step > 1f64 {
            self.positions.sample_duration * 2 * (self.positions.sample_step as u64)
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

    pub fn update_conditions(&mut self, dimensions: Option<(f64, f64)>) {
        if let Some((width, height)) = dimensions {
            self.area_width = width;
            self.area_height = height;
            self.pending_update_conditions = false;
        }

        if self.state != ControllerState::Disabled {
            debug!(
                "update_conditions {}, {}x{}",
                self.requested_duration, self.area_width, self.area_height,
            );

            {
                let waveform_renderer = &mut *self.waveform_renderer_mtx.lock().unwrap();
                waveform_renderer.update_conditions(
                    self.requested_duration,
                    self.area_width as i32,
                    self.area_height as i32,
                );
                waveform_renderer.refresh();
            }

            self.refresh_buffer();
            self.drawingarea.queue_draw();
        }
    }

    /// Refreshes conditions asynchronously.
    ///
    /// This ensures all widgets are arranged to their target positions.
    #[inline]
    pub fn update_conditions_async(&mut self) {
        if self.pending_update_conditions {
            return;
        }

        self.pending_update_conditions = true;
        self.ui_event.update_audio_rendering_cndt(None);
    }

    pub fn zoom_in(&mut self) {
        self.requested_duration /= REQ_DURATION_SCALE_FACTOR;
        if self.requested_duration >= MIN_REQ_DURATION_FOR_1000PX {
            self.update_conditions(None);
        } else {
            self.requested_duration = MIN_REQ_DURATION_FOR_1000PX;
        }
        self.seek_step = self.requested_duration / SEEK_STEP_DURATION_DIVISOR;
    }

    pub fn zoom_out(&mut self) {
        self.requested_duration *= REQ_DURATION_SCALE_FACTOR;
        if self.requested_duration <= MAX_REQ_DURATION_FOR_1000PX {
            self.update_conditions(None);
        } else {
            self.requested_duration = MAX_REQ_DURATION_FOR_1000PX;
        }
        self.seek_step = self.requested_duration / SEEK_STEP_DURATION_DIVISOR;
    }

    pub fn refresh_buffer(&self) {
        self.dbl_renderer_mtx.lock().unwrap().refresh();
    }

    pub fn tick(&mut self) {
        if self.playback_needs_refresh {
            trace!("tick forcing refresh");
            self.refresh_buffer();
        }

        if let ControllerState::Playing | ControllerState::PlayingRange(_) = self.state {
            self.redraw();
        }
    }

    pub fn draw(&mut self, _da: &gtk::DrawingArea, cr: &cairo::Context) -> Option<Timestamp> {
        cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
        cr.paint();

        if ControllerState::Disabled == self.state {
            // Not active yet, don't display
            debug!("draw still disabled => not drawing");
            return None;
        }

        // Get waveform and timestamps
        self.positions = {
            let waveform_renderer = &mut *self.waveform_renderer_mtx.lock().unwrap();
            self.playback_needs_refresh = waveform_renderer.playback_needs_refresh();

            match self.state {
                ControllerState::Playing => waveform_renderer.refresh(),
                ControllerState::PlayingRange(_) => waveform_renderer.refresh_cursor(),
                _ => (),
            }

            let (image, positions) = match waveform_renderer.image() {
                Some(image_and_positions) => image_and_positions,
                None => {
                    debug!("draw got no image");
                    return None;
                }
            };

            image.with_surface_external_context(cr, |cr, surface| {
                cr.set_source_surface(surface, -positions.offset.x, 0f64);
                cr.paint();
            });

            positions
        };

        cr.scale(1f64, 1f64);
        cr.set_source_rgb(1f64, 1f64, 0f64);

        self.adjust_waveform_text_width(cr);

        // first position
        let first_text = self.positions.offset.ts.for_humans().to_string();
        let first_text_end = if self.positions.offset.ts < ONE_HOUR {
            2f64 + self.text_metrics.limit_mn_width
        } else {
            2f64 + self.text_metrics.limit_h_width
        };
        cr.move_to(2f64, self.text_metrics.twice_font_size);
        cr.show_text(&first_text);

        // last position
        let last_text = self.positions.last.ts.for_humans().to_string();
        let last_text_start = if self.positions.last.ts < ONE_HOUR {
            2f64 + self.text_metrics.limit_mn_width
        } else {
            2f64 + self.text_metrics.limit_h_width
        };
        if self.positions.last.x - last_text_start > first_text_end + 5f64 {
            // last text won't overlap with first text
            cr.move_to(
                self.positions.last.x - last_text_start,
                self.text_metrics.twice_font_size,
            );
            cr.show_text(&last_text);
        }

        // Draw in-range chapters boundaries
        let boundaries = self.boundaries.borrow();

        let chapter_range = boundaries.range((
            Included(&self.positions.offset.ts),
            Included(&self.positions.last.ts),
        ));

        cr.set_source_rgb(0.5f64, 0.6f64, 1f64);
        cr.set_line_width(1f64);
        let boundary_y0 = self.text_metrics.twice_font_size + 5f64;
        let text_base = self.area_height - self.text_metrics.half_font_size;

        for (boundary, chapters) in chapter_range {
            if *boundary >= self.positions.offset.ts {
                let x = SampleIndexRange::from_duration(
                    *boundary - self.positions.offset.ts,
                    self.positions.sample_duration,
                )
                .as_f64()
                    / self.positions.sample_step;
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

        if let Some(cursor) = &self.positions.cursor {
            // draw current pos
            cr.set_source_rgb(1f64, 1f64, 0f64);

            let cursor_text = cursor.ts.for_humans().with_micro().to_string();
            let cursor_text_end = if cursor.ts < ONE_HOUR {
                5f64 + self.text_metrics.cursor_mn_width
            } else {
                5f64 + self.text_metrics.cursor_h_width
            };
            let cursor_text_x = if cursor.x + cursor_text_end < self.area_width {
                cursor.x + 5f64
            } else {
                cursor.x - cursor_text_end
            };
            cr.move_to(cursor_text_x, self.text_metrics.font_size);
            cr.show_text(&cursor_text);

            cr.set_line_width(1f64);
            cr.move_to(cursor.x, 0f64);
            cr.line_to(
                cursor.x,
                self.area_height - self.text_metrics.twice_font_size,
            );
            cr.stroke();

            // update other UI position
            // Note: we go through the audio controller here in order
            // to reduce position queries on the ref gst element
            match self.state {
                ControllerState::Playing => {
                    if cursor.ts >= self.last_other_ui_refresh
                        && cursor.ts <= self.last_other_ui_refresh + OTHER_UI_REFRESH_PERIOD
                    {
                        return None;
                    }
                }
                ControllerState::Paused => (),
                ControllerState::MovingBoundary(_) => (),
                _ => return None,
            }

            let cursor_ts = cursor.ts;
            self.last_other_ui_refresh = cursor_ts;

            Some(cursor_ts)
        } else {
            None
        }
    }

    pub fn motion_notify(
        &mut self,
        event_motion: &gdk::EventMotion,
    ) -> Option<(Timestamp, Timestamp)> {
        let (x, _y) = event_motion.get_position();

        match self.state {
            ControllerState::Playing => (),
            ControllerState::MovingBoundary(boundary) => {
                return self.ts_at(x).map(|position| (boundary, position));
            }
            ControllerState::Paused => {
                if let Some(boundary) = self.boundary_at(x) {
                    self.state = ControllerState::CursorAboveBoundary(boundary);
                    self.ui_event.set_cursor_double_arrow();
                }
            }
            ControllerState::CursorAboveBoundary(_) => {
                if let Some(boundary) = self.boundary_at(x) {
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
                if let Some(ts) = self.ts_at(event_button.get_position().0) {
                    match self.state {
                        ControllerState::Playing
                        | ControllerState::PlayingRange(_)
                        | ControllerState::Paused
                        | ControllerState::PausedPlayingRange(_) => {
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
                // right button => range playback in Paused state
                if let Some(start) = self.ts_at(event_button.get_position().0) {
                    let to_restore = match self.state {
                        ControllerState::Paused => {
                            self.positions.cursor.as_ref().map(|cursor| cursor.ts)
                        }
                        ControllerState::PlayingRange(to_restore)
                        | ControllerState::PausedPlayingRange(to_restore) => Some(to_restore),
                        _ => None,
                    };

                    if let Some(to_restore) = to_restore {
                        let end = start + MIN_RANGE_DURATION.max(self.positions.last.ts - start);
                        self.ui_event.play_range(start, end, to_restore);
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
