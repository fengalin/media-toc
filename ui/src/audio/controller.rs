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

use super::WaveformWithOverlay;
use crate::{audio, info::ChaptersBoundaries, main, playback, prelude::*};

const BUFFER_DURATION: Duration = Duration::from_secs(60);
const INIT_REQ_DURATION_FOR_1000PX: Duration = Duration::from_secs(4);
const MIN_REQ_DURATION_FOR_1000PX: Duration = Duration::from_nanos(1_953_125); // 4s / 2^11
const MAX_REQ_DURATION_FOR_1000PX: Duration = Duration::from_secs(32);
const REQ_DURATION_SCALE_FACTOR: u64 = 2;

const SEEK_STEP_DURATION_DIVISOR: u64 = 2;

// Range playback
const MIN_RANGE_DURATION: Duration = Duration::from_millis(100);

#[derive(Debug, PartialEq)]
pub enum State {
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

#[derive(Debug)]
pub enum AreaEvent {
    Button(gdk::EventButton),
    Leaving,
    Motion(gdk::EventMotion),
}

pub struct Controller {
    waveform_renderer_mtx: Arc<Mutex<Box<WaveformRenderer>>>,
    pub dbl_renderer_mtx: Arc<Mutex<DoubleAudioBuffer<WaveformRenderer>>>,
    pub(super) positions: Rc<RefCell<ImagePositions>>,
    boundaries: Rc<RefCell<ChaptersBoundaries>>,

    container: gtk::Box,
    pub(super) drawingarea: gtk::DrawingArea,
    zoom_in_btn: gtk::ToolButton,
    pub(super) zoom_in_action: gio::SimpleAction,
    zoom_out_btn: gtk::ToolButton,
    pub(super) zoom_out_action: gio::SimpleAction,

    pub(super) step_forward_action: gio::SimpleAction,
    pub(super) step_back_action: gio::SimpleAction,

    area_height: f64,
    area_width: f64,
    pending_update_conditions: bool,

    pub(super) state: State,
    playback_needs_refresh: bool,

    requested_duration: Duration,
    pub(crate) seek_step: Duration,

    tick_cb_id: Option<gtk::TickCallbackId>,

    ref_lbl: gtk::Label,
}

impl UIController for Controller {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let is_audio_selected = {
            let info = pipeline.info.read().unwrap();
            info.streams.is_audio_selected()
        };

        if is_audio_selected {
            self.state = State::Paused;

            // Refresh conditions asynchronously so that
            // all widgets are arranged to their target positions
            self.update_conditions_async();
        }

        // FIXME: step forward / back actions should probably be
        // defined in the InfoController. On the other hand, they
        // depend on the seek_step which is defined in Controller
        // since it depends on the zoom factor, which has to do with
        // the waveform.
        self.step_forward_action.set_enabled(true);
        self.step_back_action.set_enabled(true);
    }

    fn cleanup(&mut self) {
        self.state = State::Disabled;
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
        // Controller accesses self.boundaries as readonly
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

impl Controller {
    pub fn new(builder: &gtk::Builder, boundaries: Rc<RefCell<ChaptersBoundaries>>) -> Self {
        let dbl_waveform = DoubleWaveformRenderer::new_dbl_audio_buffer(BUFFER_DURATION);
        let waveform_renderer_mtx = dbl_waveform.exposed_buffer_mtx();

        let mut ctrl = Controller {
            waveform_renderer_mtx,
            dbl_renderer_mtx: Arc::new(Mutex::new(dbl_waveform)),
            positions: Rc::new(RefCell::new(ImagePositions::default())),
            boundaries,

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

            state: State::Disabled,
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
            &self.ref_lbl,
        )
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
            State::Playing => {
                self.state = State::Paused;
            }
            State::PlayingRange(ts) => {
                self.state = State::PausedPlayingRange(ts);
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
            State::Paused => {
                self.state = State::Playing;
            }
            State::PausedPlayingRange(ts) => {
                self.state = State::PlayingRange(ts);
            }
            _ => return,
        }

        self.waveform_renderer_mtx.lock().unwrap().release();
        self.register_tick_callback();
    }

    pub fn seek(&mut self) {
        match self.state {
            State::Playing => {
                self.state = State::SeekingPlaying;
            }
            State::Paused => self.state = State::SeekingPaused,
            State::PlayingRange(_) => {
                self.remove_tick_callback();
                self.state = State::SeekingPaused;
            }
            State::PausedPlayingRange(_) => {
                self.state = State::SeekingPaused;
            }
            _ => (),
        }

        self.waveform_renderer_mtx.lock().unwrap().seek();
    }

    pub fn seek_done(&mut self, ts: Timestamp) {
        match self.state {
            State::SeekingPlaying => {
                self.state = State::Playing;
                self.waveform_renderer_mtx.lock().unwrap().seek_done(ts);
            }
            State::SeekingPaused => {
                self.state = State::Paused;
                self.waveform_renderer_mtx.lock().unwrap().seek_done(ts);
                self.refresh();
            }
            _ => unreachable!("seek_done in {:?}", self.state),
        }
    }

    pub fn start_play_range(&mut self, to_restore: Timestamp) {
        match self.state {
            State::PlayingRange(_) => {
                self.state = State::PlayingRange(to_restore);
            }
            State::Paused | State::PausedPlayingRange(_) => {
                self.state = State::PlayingRange(to_restore);
                self.register_tick_callback();
            }
            _ => unreachable!("start_play_range in {:?}", self.state),
        }
    }

    pub fn stop_play_range(&mut self) {
        match self.state {
            State::PlayingRange(_) => {
                self.remove_tick_callback();
                self.state = State::Paused;
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
        self.tick_cb_id = Some(self.drawingarea.add_tick_callback(|_, _| {
            audio::tick();
            glib::Continue(true)
        }));
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

        if self.state != State::Disabled {
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

            self.refresh();
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
        audio::update_rendering_cndt(None);
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

    pub fn redraw(&self) {
        self.drawingarea.queue_draw();
    }

    pub fn refresh(&mut self) {
        if self.refresh_buffer() {
            self.redraw();
        }
    }

    fn refresh_buffer(&mut self) -> bool {
        self.waveform_renderer_mtx.lock().unwrap().refresh().is_ok()
    }

    pub fn tick(&mut self) {
        let mut can_redraw = true;

        if self.playback_needs_refresh {
            trace!("tick forcing refresh");
            can_redraw = self.refresh_buffer();
        }

        if can_redraw {
            if let State::Playing | State::PlayingRange(_) = self.state {
                self.redraw();
            }
        }
    }

    pub fn motion_notify(
        &mut self,
        event_motion: gdk::EventMotion,
    ) -> Option<(Timestamp, Timestamp)> {
        let (x, _y) = event_motion.get_position();

        match self.state {
            State::Playing => (),
            State::MovingBoundary(boundary) => {
                return self.ts_at(x).map(|position| (boundary, position));
            }
            State::Paused => {
                if let Some(boundary) = self.boundary_at(x) {
                    self.state = State::CursorAboveBoundary(boundary);
                    main::set_cursor_double_arrow();
                }
            }
            State::CursorAboveBoundary(_) => {
                if let Some(boundary) = self.boundary_at(x) {
                    self.state = State::CursorAboveBoundary(boundary);
                } else {
                    self.state = State::Paused;
                    main::reset_cursor();
                }
            }
            _ => (),
        }

        None
    }

    pub fn leave_drawing_area(&mut self) {
        match self.state {
            State::Playing => (),
            State::Paused => (),
            State::MovingBoundary(_) | State::CursorAboveBoundary(_) => {
                self.state = State::Paused;
                main::reset_cursor()
            }
            _ => (),
        }
    }

    pub fn button_pressed(&mut self, event_button: gdk::EventButton) {
        match event_button.get_button() {
            1 => {
                // left button
                if let Some(ts) = self.ts_at(event_button.get_position().0) {
                    match self.state {
                        State::Playing
                        | State::PlayingRange(_)
                        | State::Paused
                        | State::PausedPlayingRange(_) => {
                            playback::seek(ts, gst::SeekFlags::ACCURATE);
                        }
                        State::CursorAboveBoundary(boundary) => {
                            self.state = State::MovingBoundary(boundary);
                        }
                        _ => (),
                    }
                }
            }
            3 => {
                // right button => range playback in Paused state
                if let Some(start) = self.ts_at(event_button.get_position().0) {
                    let to_restore = match self.state {
                        State::Paused => self
                            .positions
                            .borrow()
                            .cursor
                            .as_ref()
                            .map(|cursor| cursor.ts),
                        State::PlayingRange(to_restore) | State::PausedPlayingRange(to_restore) => {
                            Some(to_restore)
                        }
                        _ => None,
                    };

                    if let Some(to_restore) = to_restore {
                        let end =
                            start + MIN_RANGE_DURATION.max(self.positions.borrow().last.ts - start);
                        playback::play_range(start, end, to_restore);
                    }
                }
            }
            _ => (),
        }
    }

    pub fn button_released(&mut self, event_button: gdk::EventButton) {
        if let State::MovingBoundary(boundary) = self.state {
            if 1 == event_button.get_button() {
                // left button
                self.state = State::CursorAboveBoundary(boundary);
            }
        }
    }
}
