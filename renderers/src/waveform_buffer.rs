use gstreamer as gst;
use log::{debug, trace, warn};
use smallvec::SmallVec;

use std::{
    any::Any,
    boxed::Box,
    sync::{Arc, Mutex},
};

use media::{
    sample_extractor::SampleExtractionState, AudioBuffer, AudioChannel, AudioChannelSide,
    DoubleAudioBuffer, Duration, SampleExtractor, SampleIndex, SampleIndexRange, Timestamp,
    INLINE_CHANNELS,
};

pub const AMPLITUDE_0_COLOR: (f64, f64, f64) = (0.5f64, 0.5f64, 0f64);

pub struct DoubleWaveformBuffer {}

impl DoubleWaveformBuffer {
    pub fn new_mutex(buffer_duration: Duration) -> Arc<Mutex<DoubleAudioBuffer>> {
        Arc::new(Mutex::new(DoubleAudioBuffer::new(
            buffer_duration,
            Box::new(WaveformBuffer::new(1)),
            Box::new(WaveformBuffer::new(2)),
        )))
    }
}

#[derive(Default)]
pub struct SamplePosition {
    pub ts: Timestamp,
    pub x: f64,
}

#[derive(Default)]
pub struct WaveformMetrics {
    pub first_ts: Timestamp,
    pub last: SamplePosition,
    pub cursor: Option<SamplePosition>,
    pub sample_duration: Duration,
    pub sample_step: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LockState {
    PlayingFirstHalf,
    PlayingSecondHalf,
    PlayingRange,
    RestoringInitialPos,
    Seeking,
}

// FIXME: update the comment
// A WaveformBuffer hosts one of the two buffers of the double buffering
// mechanism based on the SampleExtractor trait.
// It is responsible for preparing an up to date Waveform image which will be
// diplayed upon UI request. Up to date signifies that the Waveform image
// contains all the samples that can fit in the target window at the specified
// resolution for current playback timestamp.
// Whenever possible, the WaveformBuffer attempts to have the Waveform scroll
// between frames with current playback position in the middle so that the
// user can seek forward or backward around current timestamp.
#[derive(Default)]
pub struct WaveformBuffer {
    pub id: usize,
    pub is_initialized: bool,
    force_extraction: bool,

    state: SampleExtractionState,
    conditions_changed: bool,

    channel_colors: SmallVec<[(f64, f64, f64); INLINE_CHANNELS]>,

    buffer: SmallVec<[Vec<f64>; INLINE_CHANNELS]>,
    lower: SampleIndex,
    upper: SampleIndex,
    contains_eos: bool,

    previous_sample: Option<SampleIndex>,
    pub cursor_sample: SampleIndex, // The sample idx currently under the cursor (might be different from
    // current sample idx during seeks)
    cursor_ts: Timestamp,
    pub first_visible_sample: Option<SampleIndex>,
    first_visible_sample_lock: Option<(SampleIndex, LockState)>,

    // During playback, we take advantage of the running time and thus
    // the stream of incoming samples to refresh the waveform.
    // When EOS is reached, no more samples are received, so refresh
    // must be forced in order to compute the samples window to render
    pub playback_needs_refresh: bool,

    req_duration_per_1000px: Duration,
    width: i32,
    width_f: f64,
    height: i32,
    height_f: f64,
    half_range_y: f64,
    sample_value_factor: f64,
    sample_step_f: f64,
    sample_step: SampleIndexRange,
    x_step_f: f64,
    x_step: usize,

    req_sample_window: SampleIndexRange,
    half_req_sample_window: SampleIndexRange,
    eighth_req_sample_window: SampleIndexRange,
}

impl WaveformBuffer {
    pub fn new(id: usize) -> Self {
        WaveformBuffer {
            id,
            ..WaveformBuffer::default()
        }
    }

    pub fn reset(&mut self) {
        debug!("{}_reset", self.id);

        self.is_initialized = false;

        self.conditions_changed = false;

        self.channel_colors.clear();

        self.reset_sample_conditions();

        self.req_duration_per_1000px = Duration::default();
        self.width = 0;
        self.width_f = 0f64;
        self.height = 0;
        self.height_f = 0f64;
        self.half_range_y = 0f64;
        self.sample_value_factor = 0f64;
    }

    fn reset_sample_conditions(&mut self) {
        debug!("{}_reset_sample_conditions", self.id);

        self.force_extraction = false;

        self.buffer.clear();
        self.lower = SampleIndex::default();
        self.upper = SampleIndex::default();
        self.contains_eos = false;

        self.previous_sample = None;
        self.cursor_sample = SampleIndex::default();
        self.cursor_ts = Timestamp::default();
        self.first_visible_sample = None;
        self.first_visible_sample_lock = None;
        self.playback_needs_refresh = false;

        self.sample_step_f = 0f64;
        self.sample_step = SampleIndexRange::default();
        self.x_step_f = 0f64;
        self.x_step = 0;

        self.req_sample_window = SampleIndexRange::default();
        self.half_req_sample_window = SampleIndexRange::default();
        self.eighth_req_sample_window = SampleIndexRange::default();
    }

    pub fn get_limits_as_ts(&self) -> (Timestamp, Timestamp) {
        (
            self.lower.get_ts(self.state.sample_duration),
            self.upper.get_ts(self.state.sample_duration),
        )
    }

    pub fn get_half_window_duration(&self) -> Duration {
        self.half_req_sample_window
            .get_duration(self.state.sample_duration)
    }

    pub fn seek(&mut self, target: Timestamp) {
        if !self.is_initialized {
            return;
        }

        let sought_sample =
            SampleIndex::from_ts(target, self.state.sample_duration).get_aligned(self.sample_step);

        debug!(
            concat!(
                "{}_seek cursor_sample {}, sought sample {} ({}), ",
                "state: {:?}, image [{}, {}], contains_eos: {}",
            ),
            self.id,
            self.cursor_sample,
            sought_sample,
            target,
            self.state.state,
            self.lower,
            self.upper,
            self.contains_eos,
        );

        if self.state.state == gst::State::Playing {
            // stream is playing => let the cursor jump from current timestamp
            // to the sought timestamp without shifting the waveform if possible

            if let Some(first_visible_sample) = self.first_visible_sample {
                if sought_sample >= first_visible_sample
                    && sought_sample <= first_visible_sample + self.req_sample_window
                    && self.upper - self.lower >= self.req_sample_window
                {
                    // sought sample is in current window
                    // and the window is large enough for a constraint
                    // => lock the first sample so that the cursor appears
                    // at the sought idx without abrutely scrolling
                    // the waveform.
                    self.first_visible_sample_lock =
                        Some((first_visible_sample, LockState::Seeking));
                } else {
                    self.first_visible_sample_lock = None;
                }
            } else {
                self.first_visible_sample_lock = None;
            }
        } else {
            // not playing
            self.first_visible_sample = match self.first_visible_sample_lock.take() {
                Some((first_visible_sample, LockState::PlayingRange)) => {
                    // Range is complete => we are restoring the initial position
                    self.first_visible_sample_lock =
                        Some((first_visible_sample, LockState::RestoringInitialPos));
                    Some(first_visible_sample)
                }
                _ => None,
            };
        }

        self.previous_sample = None;
        self.cursor_sample = sought_sample;
    }

    pub fn seek_complete(&mut self) {
        self.previous_sample = None;

        if let Some((first_visible_sample_lock, LockState::Seeking)) =
            self.first_visible_sample_lock
        {
            let lock_state =
                if self.cursor_sample < first_visible_sample_lock + self.half_req_sample_window {
                    LockState::PlayingFirstHalf
                } else {
                    LockState::PlayingSecondHalf
                };
            self.first_visible_sample_lock = Some((first_visible_sample_lock, lock_state));
        }
    }

    pub fn start_play_range(&mut self) {
        if !self.is_initialized {
            return;
        }

        if let Some(first_visible_sample) = self.first_visible_sample {
            self.first_visible_sample_lock = Some((first_visible_sample, LockState::PlayingRange));
        }
    }

    fn refresh_ts(&mut self, last_frame_ts: Timestamp, next_frame_ts: Timestamp) {
        match self.first_visible_sample_lock {
            Some((_first_visible_sample_lock, LockState::Seeking)) => (),
            _ => {
                let (ts, mut sample) = self.get_current_sample(last_frame_ts, next_frame_ts);

                if self.get_extraction_state().is_stable {
                    // after seek stabilization complete
                    self.previous_sample = Some(self.cursor_sample);
                }

                if self.contains_eos && sample >= self.upper {
                    sample = self.upper;
                    sample
                        .try_dec()
                        .expect("adjusting cursor_sample to last sample in stream");
                }
                self.cursor_sample = sample;
                self.cursor_ts = ts;
            }
        }
    }

    // Update to current timestamp and compute the first sample to display.
    pub fn update_first_visible_sample(
        &mut self,
        last_frame_ts: Timestamp,
        next_frame_ts: Timestamp,
    ) {
        self.first_visible_sample = if self.is_initialized {
            self.refresh_ts(last_frame_ts, next_frame_ts);

            if self.cursor_sample >= self.lower {
                // current sample appears after first buffer sample
                if let Some((first_visible_sample_lock, lock_state)) =
                    self.first_visible_sample_lock
                {
                    // There is a scrolling lock constraint
                    match lock_state {
                        LockState::PlayingFirstHalf => {
                            if self.cursor_sample - first_visible_sample_lock
                                < self.half_req_sample_window
                            {
                                // still in first half => keep first visible sample lock
                                self.first_visible_sample_lock =
                                    Some((first_visible_sample_lock, lock_state));
                                Some(first_visible_sample_lock)
                            } else {
                                // No longer in first half => center cursor
                                self.first_visible_sample_lock = None;
                                Some(
                                    self.lower
                                        .max(self.cursor_sample - self.half_req_sample_window),
                                )
                            }
                        }
                        LockState::PlayingSecondHalf => {
                            if self.cursor_sample - first_visible_sample_lock
                                > self.half_req_sample_window
                            {
                                // still in second half
                                if first_visible_sample_lock + self.req_sample_window < self.upper {
                                    // and there is still overhead
                                    // => progressively get cursor back to center
                                    match self.previous_sample {
                                        Some(previous_sample) => {
                                            let previous_offset =
                                                previous_sample - first_visible_sample_lock;
                                            let delta_cursor = self.cursor_sample - previous_sample;
                                            let next_lower = self.lower.max(
                                                self.cursor_sample - previous_offset + delta_cursor,
                                            );

                                            self.first_visible_sample_lock =
                                                Some((next_lower, LockState::PlayingSecondHalf));
                                            Some(next_lower)
                                        }
                                        None => {
                                            // Wait until we can get a reference on the sample
                                            // increment wrt the frame refreshing rate
                                            return;
                                        }
                                    }
                                } else {
                                    // Not enough overhead to get cursor back to center
                                    // Follow toward the last sample
                                    let next_lower = if self.lower + self.req_sample_window
                                        < self.upper
                                    {
                                        // buffer window is larger than req_sample_window
                                        // set last buffer to the right
                                        let next_lower = self.upper - self.req_sample_window;
                                        if !self.contains_eos {
                                            // but keep the constraint in case more samples
                                            // are added afterward
                                            self.first_visible_sample_lock =
                                                Some((next_lower, LockState::PlayingSecondHalf));
                                        } else {
                                            // reached EOS => don't expect returning to center
                                            self.first_visible_sample_lock = None;
                                        }

                                        next_lower
                                    } else {
                                        // buffer window is smaller than req_sample_window
                                        // set first sample to the left
                                        self.first_visible_sample_lock = None;
                                        self.lower
                                    };

                                    Some(next_lower)
                                }
                            } else {
                                // No longer in second half => center cursor
                                self.first_visible_sample_lock = None;
                                Some(
                                    self.lower
                                        .max(self.cursor_sample - self.half_req_sample_window),
                                )
                            }
                        }
                        LockState::PlayingRange => {
                            // keep origin on the first sample upon seek
                            Some(first_visible_sample_lock)
                        }
                        LockState::RestoringInitialPos => {
                            self.first_visible_sample_lock = None;
                            Some(first_visible_sample_lock)
                        }
                        LockState::Seeking => {
                            // Wait until new sample delta can be computed
                            return;
                        }
                    }
                } else if self.cursor_sample + self.half_req_sample_window <= self.upper {
                    // cursor_sample fits in the first half of the window with last sample further
                    if self.cursor_sample > self.lower + self.half_req_sample_window {
                        // cursor_sample can be centered
                        Some(self.cursor_sample - self.half_req_sample_window)
                    } else {
                        // cursor_sample before half of displayable window
                        // set origin to the first sample in the buffer
                        // current sample will be displayed between the origin
                        // and the center
                        Some(self.lower)
                    }
                } else if self.cursor_sample >= self.upper {
                    // cursor_sample appears after buffer's last sample
                    debug!(
                        concat!(
                            "{}_update_first_visible_sample ",
                            "cursor_sample {} appears after buffer upper bound {}",
                        ),
                        self.id, self.cursor_sample, self.upper,
                    );
                    if self.upper + self.req_sample_window >= self.cursor_sample {
                        // rebase buffer attempting to keep in range
                        // even if samples are not rendered yet
                        if self.cursor_sample > self.lower + self.req_sample_window {
                            Some(self.cursor_sample - self.req_sample_window)
                        } else {
                            Some(self.lower)
                        }
                    } else {
                        // cursor no longer in range, 2 cases:
                        // - seeking forward
                        // - zoomed-in too much to keep up with the audio stream
                        None
                    }
                } else if self.lower + self.req_sample_window < self.upper {
                    // buffer window is larger than req_sample_window
                    // set last buffer to the right
                    Some(self.upper - self.req_sample_window)
                } else {
                    // buffer window is smaller than req_sample_window
                    // set first sample to the left
                    Some(self.lower)
                }
            } else if self.cursor_sample + self.req_sample_window > self.lower {
                // cursor is close enough to current buffer
                // => render what can be rendered
                Some(self.lower)
            } else {
                // cursor_sample appears before buffer's first sample
                // => wait until situation clarifies
                debug!(
                    concat!(
                        "{}_update_first_visible_sample cursor_sample {} ",
                        "appears before buffer's first sample {}",
                    ),
                    self.id, self.cursor_sample, self.lower
                );
                None
            }
        } else {
            debug!("{}_update_first_visible_sample buffer not ready", self.id);
            None
        };
    }

    // Update rendering conditions
    pub fn update_conditions(&mut self, duration_per_1000px: Duration, width: i32, height: i32) {
        let (duration_changed, mut scale_num, mut scale_denom) =
            if duration_per_1000px == self.req_duration_per_1000px {
                (false, 0, 0)
            } else {
                let prev_duration = self.req_duration_per_1000px;
                self.req_duration_per_1000px = duration_per_1000px;
                debug!(
                    "{}_update_conditions duration/1000px {} -> {}",
                    self.id, prev_duration, self.req_duration_per_1000px,
                );
                self.update_sample_step();
                (
                    true,
                    duration_per_1000px.as_usize(),
                    prev_duration.as_usize(),
                )
            };

        let width_changed = if width == self.width {
            false
        } else {
            if self.width != 0 {
                scale_num = width as usize;
                scale_denom = self.width as usize;
            }

            debug!(
                "{}_update_conditions width {} -> {}",
                self.id, self.width, width
            );

            self.width = width;
            self.width_f = f64::from(width);
            true
        };

        if height != self.height {
            debug!(
                "{}_update_conditions height {} -> {}",
                self.id, self.height, height
            );

            self.height = height;
            self.height_f = f64::from(height);
            self.half_range_y = self.height_f / 2f64;
            self.sample_value_factor = self.half_range_y / f64::from(std::i16::MIN);

            self.conditions_changed = true;
            self.force_extraction = true;
        };

        if duration_changed || width_changed {
            self.conditions_changed = true;
            self.force_extraction = true;

            self.update_sample_window();

            // update first sample in order to match new conditions
            if scale_num != 0 {
                self.first_visible_sample = match self.first_visible_sample {
                    Some(first_visible_sample) => {
                        let new_first_visible_sample = first_visible_sample
                            + self.req_sample_window.get_scaled(scale_num, scale_denom);

                        if new_first_visible_sample > self.sample_step {
                            let lower = self.lower;
                            let new_first_visible_sample = if new_first_visible_sample > lower {
                                new_first_visible_sample
                            } else {
                                lower
                            };

                            debug!(
                                concat!(
                                    "{}_rebase range [{}, {}], window {}, ",
                                    "first {} -> {}, sample_step {}, cursor_sample {}",
                                ),
                                self.id,
                                self.lower,
                                self.upper,
                                self.req_sample_window,
                                first_visible_sample,
                                new_first_visible_sample,
                                self.sample_step,
                                self.cursor_sample,
                            );

                            if let Some((_first_visible_sample_lock, lock_state)) =
                                self.first_visible_sample_lock.take()
                            {
                                // There is a first visible sample constraint
                                // => adapt it to match the new zoom
                                let lock_state = match lock_state {
                                    LockState::PlayingFirstHalf | LockState::PlayingSecondHalf => {
                                        if self.cursor_sample
                                            > new_first_visible_sample + self.half_req_sample_window
                                        {
                                            LockState::PlayingFirstHalf
                                        } else {
                                            LockState::PlayingSecondHalf
                                        }
                                    }
                                    other => other,
                                };
                                self.first_visible_sample_lock =
                                    Some((new_first_visible_sample, lock_state));
                            }

                            Some(new_first_visible_sample)
                        } else {
                            // first_visible_sample can be snapped to the beginning
                            // => have refresh and update_first_visible_sample
                            // compute the best range for this situation
                            None
                        }
                    }
                    None => None,
                };
            }
        }
    }

    fn update_sample_step(&mut self) {
        // compute a sample step which will produce an integer number of
        // samples per pixel or an integer number of pixels per samples

        self.sample_step_f = if self.req_duration_per_1000px >= self.state.duration_per_1000_samples
        {
            (self.req_duration_per_1000px.as_f64() / self.state.duration_per_1000_samples.as_f64())
                .floor()
        } else {
            1f64 / (self.state.duration_per_1000_samples.as_f64()
                / self.req_duration_per_1000px.as_f64())
            .ceil()
        };
        self.sample_step = (self.sample_step_f as usize).max(1).into();

        self.x_step_f = if self.sample_step_f < 1f64 {
            (1f64 / self.sample_step_f).round()
        } else {
            1f64
        };
        self.x_step = self.x_step_f as usize;
    }

    fn update_sample_window(&mut self) {
        // force sample window to an even number of samples so that the cursor can be centered
        // and make sure to cover at least the requested width
        let half_req_sample_window = (self.sample_step_f * self.width_f / 2f64) as usize;
        let req_sample_window = half_req_sample_window * 2;

        if req_sample_window != self.req_sample_window.as_usize() {
            debug!(
                "{}_update_sample_window smpl.window prev. {}, new {} (sample_step: {})",
                self.id, self.req_sample_window, req_sample_window, self.sample_step
            );

            self.req_sample_window = req_sample_window.into();
            self.half_req_sample_window = half_req_sample_window.into();
            self.eighth_req_sample_window = (half_req_sample_window / 4).into();

            self.is_initialized = true;
            self.conditions_changed = true;
        }
    }

    fn draw(&mut self, cr: &cairo::Context, first_index: usize, last_index: usize, last_x: f64) {
        // Draw axis
        cr.set_line_width(1f64);
        cr.set_source_rgb(
            AMPLITUDE_0_COLOR.0,
            AMPLITUDE_0_COLOR.1,
            AMPLITUDE_0_COLOR.2,
        );

        cr.move_to(0f64, self.half_range_y);
        cr.line_to(last_x, self.half_range_y);
        cr.stroke();

        // Draw waveform
        if self.x_step == 1 {
            cr.set_line_width(1f64);
        } else if self.x_step < 4 {
            cr.set_line_width(1.5f64);
        } else {
            cr.set_line_width(2f64);
        }

        for (channel_idx, channel) in self.buffer.iter().enumerate() {
            if let Some(&(red, green, blue)) = self.channel_colors.get(channel_idx) {
                cr.set_source_rgb(red, green, blue);
            } else {
                warn!(
                    "{}_draw_samples no color for channel {}",
                    self.id, channel_idx
                );
            }

            let mut x = 0f64;
            cr.move_to(0f64, channel[first_index]);

            for y in channel[first_index + 1..last_index].iter() {
                x += self.x_step_f;
                cr.line_to(x, *y);
            }

            cr.stroke();
        }
    }

    pub fn render(&mut self, cr: &cairo::Context) -> Option<WaveformMetrics> {
        let first_visible_sample = self.first_visible_sample?;

        let sample_duration = self.state.sample_duration;

        let first_index = (first_visible_sample - self.lower).get_step_range(self.sample_step);
        let first_ts = first_visible_sample.get_ts(sample_duration);

        let (last_index, last) = if first_visible_sample < self.upper {
            let visible_sample_range = self.upper - first_visible_sample;
            if visible_sample_range > self.req_sample_window {
                (
                    self.req_sample_window.get_step_range(self.sample_step) + first_index,
                    SamplePosition {
                        ts: (first_visible_sample + self.req_sample_window)
                            .get_ts(self.state.sample_duration),
                        x: self.width_f,
                    },
                )
            } else {
                // FIXME: check these are really the last ones, especially for ts
                let delta_index = visible_sample_range.get_step_range(self.sample_step);
                (
                    delta_index + first_index,
                    SamplePosition {
                        ts: self.upper.get_ts(self.state.sample_duration),
                        x: delta_index as f64 * self.x_step_f,
                    },
                )
            }
        } else {
            // FIXME: is this really possible?
            println!("first_visible_sample >= self.upper");
            return None;
        };

        if last_index < first_index + 1 {
            return None;
        }

        self.draw(cr, first_index, last_index, last.x);

        let cursor = if self.cursor_sample >= first_visible_sample
            && self.cursor_sample <= first_visible_sample + self.req_sample_window
        {
            let delta_index =
                (self.cursor_sample - first_visible_sample).get_step_range(self.sample_step);
            Some(SamplePosition {
                ts: self.cursor_ts,
                x: delta_index as f64 * self.x_step_f,
            })
        } else {
            None
        };

        Some(WaveformMetrics {
            first_ts,
            last,
            cursor,
            sample_duration,
            sample_step: self.sample_step_f,
        })
    }

    fn get_sample_range(&mut self, audio_buffer: &AudioBuffer) -> (SampleIndex, SampleIndex) {
        let extraction_range = if audio_buffer.upper - audio_buffer.lower <= self.req_sample_window
        {
            // can use the full window
            trace!(
                "{}_get_sample_range using full window, range [{}, {}]",
                self.id,
                audio_buffer.lower,
                audio_buffer.upper,
            );

            self.first_visible_sample = None;
            self.first_visible_sample_lock = None;

            Some((audio_buffer.lower, audio_buffer.upper))
        } else if self.cursor_sample <= audio_buffer.lower
            || self.cursor_sample >= audio_buffer.upper
        {
            // cursor sample out of the buffer's range
            trace!(
                concat!(
                    "{}_get_sample_range cursor not in the window: first_visible_sample ",
                    "{:?}, cursor {}, merged range [{}, {}]",
                ),
                self.id,
                self.first_visible_sample,
                self.cursor_sample,
                audio_buffer.lower,
                audio_buffer.upper,
            );

            self.first_visible_sample = None;
            self.first_visible_sample_lock = None;

            // Use defaults
            None
        } else {
            match self.first_visible_sample {
                Some(first_visible_sample) => {
                    if self.cursor_sample >= first_visible_sample
                        && self.cursor_sample < first_visible_sample + self.req_sample_window
                    {
                        // cursor is in the window => keep it
                        trace!(
                            concat!(
                                "{}_get_sample_range cursor in the window: first_visible_sample ",
                                "{}, cursor {}, range [{}, {}]",
                            ),
                            self.id,
                            first_visible_sample,
                            self.cursor_sample,
                            audio_buffer.lower,
                            audio_buffer.upper,
                        );

                        Some((
                            first_visible_sample,
                            audio_buffer.upper.min(
                                first_visible_sample
                                    + self.req_sample_window
                                    + self.eighth_req_sample_window,
                            ),
                        ))
                    } else {
                        debug!(
                            concat!(
                                "{}_get_sample_range first_visible_sample ",
                                "{} and cursor {} not in the same range [{}, {}]",
                            ),
                            self.id,
                            first_visible_sample,
                            self.cursor_sample,
                            audio_buffer.lower,
                            audio_buffer.upper,
                        );

                        match self.first_visible_sample_lock.take() {
                            None => {
                                if self.playback_needs_refresh {
                                    // refresh to the full available range

                                    Some((
                                        first_visible_sample,
                                        audio_buffer
                                            .upper
                                            .min(first_visible_sample + self.req_sample_window),
                                    ))
                                } else {
                                    self.first_visible_sample = None;

                                    // use defaults
                                    None
                                }
                            }
                            Some((first_visible_sample, lock_state)) => match lock_state {
                                LockState::PlayingFirstHalf
                                | LockState::PlayingSecondHalf
                                | LockState::Seeking => {
                                    self.first_visible_sample = None;
                                    // use defaults
                                    None
                                }
                                LockState::PlayingRange | LockState::RestoringInitialPos => {
                                    // keep position
                                    self.first_visible_sample_lock =
                                        Some((first_visible_sample, lock_state));

                                    Some((
                                        first_visible_sample,
                                        audio_buffer
                                            .upper
                                            .min(first_visible_sample + self.req_sample_window),
                                    ))
                                }
                            },
                        }
                    }
                }
                None => {
                    if self.cursor_sample > audio_buffer.lower + self.half_req_sample_window
                        && self.cursor_sample < audio_buffer.upper
                    {
                        // cursor can be centered or is in second half of the window
                        if self.cursor_sample + self.half_req_sample_window < audio_buffer.upper {
                            // cursor can be centered
                            debug!(
                                "{}_get_sample_range centering cursor: {}",
                                self.id, self.cursor_sample,
                            );

                            Some((
                                self.cursor_sample - self.half_req_sample_window,
                                audio_buffer.upper.min(
                                    self.cursor_sample
                                        + self.half_req_sample_window
                                        + self.eighth_req_sample_window,
                                ),
                            ))
                        } else {
                            // cursor in second half
                            debug!(
                                "{}_get_sample_range cursor: {} in second half",
                                self.id, self.cursor_sample,
                            );

                            // attempt to get an optimal range
                            if audio_buffer.upper >= audio_buffer.lower + self.req_sample_window {
                                Some((
                                    audio_buffer.upper - self.req_sample_window,
                                    audio_buffer.upper,
                                ))
                            } else {
                                // use defaults
                                None
                            }
                        }
                    } else {
                        debug!(
                            "{}_get_sample_range cursor {} in first half or before range [{}, {}]",
                            self.id, self.cursor_sample, audio_buffer.lower, audio_buffer.upper,
                        );

                        // use defaults
                        None
                    }
                }
            }
        };

        extraction_range.unwrap_or_else(|| {
            (
                audio_buffer.lower,
                audio_buffer.upper.min(
                    audio_buffer.lower + self.req_sample_window + self.eighth_req_sample_window,
                ),
            )
        })
    }

    // TODO: move these in a new WaveformBuffer struct and rename current as WaveformRenderer
    fn extract_waveform_samples(
        &mut self,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
    ) {
        if !self.is_initialized {
            return;
        }

        // Align requested lower and upper sample bounds in order to keep
        // a steady offset between redraws. This allows using the same samples
        // for a given req_step_duration and avoiding flickering
        // between redraws.
        let mut lower = lower.get_aligned(self.sample_step);
        if lower < audio_buffer.lower {
            // first sample might be smaller than audio_buffer.lower
            // due to alignement on sample_step
            lower += self.sample_step;
        }

        // When audio_buffer contains eod, we won't be called again => extract all we can get
        self.contains_eos = audio_buffer.contains_eos();
        let upper = if !self.contains_eos {
            upper.get_aligned(self.sample_step)
        } else {
            audio_buffer.upper.get_aligned(self.sample_step)
        };

        if lower >= upper {
            // can't draw current range
            // reset buffer state
            debug!(
                "{}_render lower {} greater or equal upper {}",
                self.id, lower, upper
            );

            self.lower = SampleIndex::default();
            self.upper = SampleIndex::default();
            return;
        }

        if upper < lower + self.sample_step {
            debug!(
                "{}_extract_waveform_samples range [{}, {}] too small for sample_step: {}",
                self.id, lower, upper, self.sample_step,
            );
            return;
        }

        if !self.force_extraction
            && !self.contains_eos
            && upper <= self.upper
            && lower >= self.lower
        {
            // target extraction fits in previous extraction
            return;
        }

        self.buffer.clear();
        for channel in 0..audio_buffer.channels {
            self.buffer.push(
                audio_buffer
                    .try_iter(lower, upper, channel, self.sample_step)
                    .unwrap_or_else(|err| panic!("{}_extract_waveform_samples: {}", self.id, err))
                    .map(|channel_value| {
                        f64::from(i32::from(channel_value.as_i16()) - i32::from(std::i16::MAX))
                            * self.sample_value_factor
                    })
                    .collect(),
            );
        }

        self.lower = lower;
        self.upper = upper;
    }
}

impl SampleExtractor for WaveformBuffer {
    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn get_extraction_state(&self) -> &SampleExtractionState {
        &self.state
    }

    fn get_extraction_state_mut(&mut self) -> &mut SampleExtractionState {
        &mut self.state
    }

    fn get_lower(&self) -> SampleIndex {
        self.first_visible_sample.map_or(self.lower, |sample| {
            if sample > self.half_req_sample_window {
                sample - self.half_req_sample_window
            } else {
                sample
            }
        })
    }

    fn get_requested_sample_window(&self) -> Option<SampleIndexRange> {
        if self.req_sample_window == SampleIndexRange::default() {
            None
        } else {
            Some(self.req_sample_window)
        }
    }

    fn cleanup(&mut self) {
        // clear for reuse
        debug!("{}_cleanup", self.id);

        self.state.cleanup();
        self.reset();
    }

    fn set_sample_duration(&mut self, per_sample: Duration, per_1000_samples: Duration) {
        self.reset_sample_conditions();

        debug!("{}_set_sample_duration per_sample {}", self.id, per_sample);
        self.state.sample_duration = per_sample;
        self.state.duration_per_1000_samples = per_1000_samples;
        self.update_sample_step();
        self.update_sample_window();
    }

    fn set_channels(&mut self, channels: &[AudioChannel]) {
        debug!("{}_set_channels {}", self.id, channels.len());

        for channel in channels.iter().take(INLINE_CHANNELS) {
            self.channel_colors.push(match channel.side {
                AudioChannelSide::Center => (0f64, channel.factor, 0f64),
                AudioChannelSide::Left => (channel.factor, channel.factor, channel.factor),
                AudioChannelSide::NotLocalized => (0f64, 0f64, channel.factor),
                AudioChannelSide::Right => (channel.factor, 0f64, 0f64),
            });
        }
    }

    fn switch_to_paused(&mut self) {
        match self.first_visible_sample_lock.take() {
            None => self.first_visible_sample = None,
            Some((first_visible_sample, lock_state)) => match lock_state {
                LockState::PlayingFirstHalf | LockState::PlayingSecondHalf | LockState::Seeking => {
                    self.first_visible_sample = None
                }
                LockState::PlayingRange | LockState::RestoringInitialPos =>
                // don't drop first_visible_sample & first_visible_sample_lock
                {
                    self.first_visible_sample_lock = Some((first_visible_sample, lock_state))
                }
            },
        }
    }

    fn update_concrete_state(&mut self, other: &mut dyn SampleExtractor) {
        let other = other.as_mut_any().downcast_mut::<WaveformBuffer>().unwrap();

        self.previous_sample = other.previous_sample;
        self.cursor_sample = other.cursor_sample;
        self.cursor_ts = other.cursor_ts;
        self.first_visible_sample = other.first_visible_sample;
        self.first_visible_sample_lock = other.first_visible_sample_lock;

        // playback_needs_refresh is set during extract_samples
        // so other must be updated with self status
        other.playback_needs_refresh = self.playback_needs_refresh;

        if other.conditions_changed {
            debug!("{}_update_concrete_state conditions_changed", self.id);
            self.req_duration_per_1000px = other.req_duration_per_1000px;
            self.width = other.width;
            self.width_f = other.width_f;
            self.height = other.height;
            self.height_f = other.height_f;
            self.half_range_y = other.half_range_y;
            self.sample_value_factor = other.sample_value_factor;
            self.sample_step = other.sample_step;
            self.sample_step_f = other.sample_step_f;
            self.x_step = other.x_step;
            self.x_step_f = other.x_step_f;
            self.req_sample_window = other.req_sample_window;
            self.half_req_sample_window = other.half_req_sample_window;
            self.eighth_req_sample_window = other.eighth_req_sample_window;

            self.is_initialized = other.is_initialized;
            other.conditions_changed = false;
        } // else: other has nothing new

        self.state.base_ts = other.state.base_ts;
        self.state.last_ts = other.state.last_ts;
        self.state.is_stable = other.state.is_stable;
    }

    // This is the entry point for the waveform update.
    // This function tries to merge the samples added to the AudioBuffer
    // since last extraction and adapts to the evolving conditions of
    // the playback position and target rendering dimensions and
    // resolution.
    fn extract_samples(&mut self, audio_buffer: &AudioBuffer) {
        if !self.is_initialized {
            // conditions not defined yet
            return;
        }

        self.playback_needs_refresh = if audio_buffer.contains_eos() && !self.contains_eos {
            // there won't be any refresh on behalf of audio_buffer
            // and image will still need more sample if playback continues
            debug!("{}_extract_samples setting playback_needs_refresh", self.id);

            true
        } else {
            if self.playback_needs_refresh {
                debug!(
                    "{}_extract_samples resetting playback_needs_refresh",
                    self.id
                );
            }
            false
        };

        // Get the available sample range considering both
        // the waveform image and the AudioBuffer
        let (lower, upper) = self.get_sample_range(audio_buffer);
        self.extract_waveform_samples(audio_buffer, lower, upper);
    }

    fn refresh(&mut self, audio_buffer: &AudioBuffer) {
        if self.is_initialized {
            // Note: current state is up to date (updated from DoubleAudioBuffer)

            let (lower, upper) = self.get_sample_range(audio_buffer);
            self.extract_waveform_samples(audio_buffer, lower, upper);

            self.playback_needs_refresh = {
                if self.playback_needs_refresh && self.contains_eos {
                    debug!("{}_refresh resetting playback_needs_refresh", self.id);
                }

                !self.contains_eos
            };
        } else {
            debug!("{}_refresh not initialized yet", self.id);
        }
    }
}
