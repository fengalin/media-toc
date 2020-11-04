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

use media::{DoubleAudioBuffer, Timestamp, QUEUE_SIZE};
use metadata::{Duration, MediaInfo};
use renderers::{DoubleWaveformRenderer, ImagePositions, WaveformRenderer};

use super::{
    ChaptersBoundaries, PlaybackPipeline, UIController, UIEventSender, WaveformWithOverlay,
};

const BUFFER_DURATION: Duration = Duration::from_secs(60);
const INIT_REQ_DURATION_FOR_1000PX: Duration = Duration::from_secs(4);
const MIN_REQ_DURATION_FOR_1000PX: Duration = Duration::from_nanos(1_953_125); // 4s / 2^11
const MAX_REQ_DURATION_FOR_1000PX: Duration = Duration::from_secs(32);
const REQ_DURATION_SCALE_FACTOR: u64 = 2;

const SEEK_STEP_DURATION_DIVISOR: u64 = 2;

// Range playback
const MIN_RANGE_DURATION: Duration = Duration::from_millis(100);

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

pub struct AudioController {
    waveform_renderer_mtx: Arc<Mutex<Box<WaveformRenderer>>>,
    pub dbl_renderer_mtx: Arc<Mutex<DoubleAudioBuffer<WaveformRenderer>>>,
    pub(super) positions: Rc<RefCell<ImagePositions>>,
    boundaries: Rc<RefCell<ChaptersBoundaries>>,

    ui_event: UIEventSender,

    container: gtk::Box,
    pub(super) drawingarea: gtk::DrawingArea,
    zoom_in_btn: gtk::ToolButton,
    pub(super) zoom_in_action: gio::SimpleAction,
    zoom_out_btn: gtk::ToolButton,
    pub(super) zoom_out_action: gio::SimpleAction,

    pub(super) step_forward_action: gio::SimpleAction,
    pub(super) step_back_action: gio::SimpleAction,

    pub(super) area_height: f64,
    pub(super) area_width: f64,
    pub(super) pending_update_conditions: bool,

    pub(super) state: ControllerState,
    playback_needs_refresh: bool,

    requested_duration: Duration,
    pub(super) seek_step: Duration,

    tick_cb_id: Option<gtk::TickCallbackId>,

    ref_lbl: gtk::Label,
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
        *self.positions.borrow_mut() = ImagePositions::default();
        // AudioController accesses self.boundaries as readonly
        // clearing it is under the responsiblity of ChapterTreeManager
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
            waveform_renderer_mtx,
            dbl_renderer_mtx: Arc::new(Mutex::new(dbl_waveform)),
            positions: Rc::new(RefCell::new(ImagePositions::default())),
            boundaries,

            ui_event,

            container: builder.get_object("audio-container").unwrap(),
            drawingarea: builder.get_object("audio-drawingarea").unwrap(),
            zoom_in_btn: builder.get_object("audio_zoom_in-toolbutton").unwrap(),
            zoom_in_action: gio::SimpleAction::new("zoom_in", None),
            zoom_out_btn: builder.get_object("audio_zoom_out-toolbutton").unwrap(),
            zoom_out_action: gio::SimpleAction::new("zoom_out", None),

            step_forward_action: gio::SimpleAction::new("step_forward", None),
            step_back_action: gio::SimpleAction::new("step_back", None),

            area_height: 0f64,
            area_width: 0f64,
            pending_update_conditions: false,

            state: ControllerState::Disabled,
            playback_needs_refresh: false,

            requested_duration: INIT_REQ_DURATION_FOR_1000PX,
            seek_step: INIT_REQ_DURATION_FOR_1000PX / SEEK_STEP_DURATION_DIVISOR,

            tick_cb_id: None,

            ref_lbl: builder.get_object("title-caption").unwrap(),
        };

        ctrl.cleanup();

        ctrl
    }

    pub fn waveform_with_overlay(&self) -> WaveformWithOverlay {
        WaveformWithOverlay::new(
            &self.waveform_renderer_mtx,
            &self.positions,
            &self.boundaries,
            &self.ui_event,
            &self.ref_lbl,
        )
    }

    pub fn redraw(&self) {
        self.drawingarea.queue_draw();
    }

    pub fn refresh(&mut self) {
        self.refresh_buffer();
        if self.waveform_renderer_mtx.lock().unwrap().refresh().is_ok() {
            self.drawingarea.queue_draw();
        }
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

        self.waveform_renderer_mtx.lock().unwrap().seek();
    }

    pub fn seek_done(&mut self, ts: Timestamp) {
        match self.state {
            ControllerState::SeekingPlaying => {
                self.state = ControllerState::Playing;
                self.waveform_renderer_mtx.lock().unwrap().seek_done(ts);
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
            let positions = self.positions.borrow();
            let ts = positions.offset.ts
                + positions.sample_duration * ((x * positions.sample_step).round() as u64);
            if ts <= positions.last.ts {
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

        let tolerance = {
            let positions = self.positions.borrow();
            if positions.sample_step > 1f64 {
                positions.sample_duration * 2 * (positions.sample_step as u64)
            } else {
                Duration::from_nanos(1)
            }
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
                let _ = waveform_renderer.refresh();
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
                        ControllerState::Paused => self
                            .positions
                            .borrow()
                            .cursor
                            .as_ref()
                            .map(|cursor| cursor.ts),
                        ControllerState::PlayingRange(to_restore)
                        | ControllerState::PausedPlayingRange(to_restore) => Some(to_restore),
                        _ => None,
                    };

                    if let Some(to_restore) = to_restore {
                        let end =
                            start + MIN_RANGE_DURATION.max(self.positions.borrow().last.ts - start);
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
