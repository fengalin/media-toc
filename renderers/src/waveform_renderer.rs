use gstreamer as gst;
use log::{debug, trace};

use std::{
    boxed::Box,
    sync::{Arc, Mutex},
};

use media::{
    sample_extractor::SampleExtractionState, AudioBuffer, AudioChannel, DoubleAudioBuffer,
    Duration, SampleExtractor, SampleIndex, SampleIndexRange, Timestamp,
};

use super::{WaveformBuffer, WaveformTracer};

pub struct DoubleWaveformRenderer {}

impl DoubleWaveformRenderer {
    pub fn new_mutex(buffer_duration: Duration) -> Arc<Mutex<DoubleAudioBuffer<WaveformRenderer>>> {
        Arc::new(Mutex::new(DoubleAudioBuffer::new(
            buffer_duration,
            Box::new(WaveformRenderer::new(1)),
            Box::new(WaveformRenderer::new(2)),
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

// A WaveformRenderer hosts one of the two buffers of the double buffering
// mechanism based on the SampleExtractor trait.
// It is responsible for preparing an up to date buffer suitable to render
// the Waveform upon UI request. Up to date signifies that the Waveform buffer
// contains all the samples that can fit in the target window at the specified
// resolution for current playback timestamp.
// Whenever possible, the WaveformRenderer attempts to have the Waveform scroll
// between frames with current playback position in the middle so that the
// user can seek forward or backward around current timestamp.
#[derive(Default)]
pub struct WaveformRenderer {
    pub id: usize,
    pub is_initialized: bool,

    state: SampleExtractionState,
    conditions_changed: bool,

    buffer: WaveformBuffer,

    previous_sample: Option<SampleIndex>,
    pub cursor_sample: SampleIndex,
    cursor_ts: Timestamp,
    pub first_visible_sample: Option<SampleIndex>,
    first_visible_sample_lock: Option<(SampleIndex, LockState)>,

    // During playback, we take advantage of the running time and thus
    // the stream of incoming samples to refresh the waveform.
    // When EOS is reached, no more samples are received, so refresh
    // must be forced in order to compute the samples window to render
    pub playback_needs_refresh: bool,

    req_duration_per_1000px: Duration,
    tracer: WaveformTracer,

    req_sample_window: SampleIndexRange,
    half_req_sample_window: SampleIndexRange,
    eighth_req_sample_window: SampleIndexRange,

    pub sample_step_f: f64,
    pub sample_step: SampleIndexRange,
}

impl WaveformRenderer {
    pub fn new(id: usize) -> Self {
        WaveformRenderer {
            id,
            buffer: WaveformBuffer::new(id),
            tracer: WaveformTracer::new(id),
            ..WaveformRenderer::default()
        }
    }

    pub fn reset(&mut self) {
        debug!("{}_reset", self.id);

        self.is_initialized = false;
        self.conditions_changed = false;

        self.tracer.reset();

        self.reset_sample_conditions();
        self.req_duration_per_1000px = Duration::default();
    }

    fn reset_sample_conditions(&mut self) {
        debug!("{}_reset_sample_conditions", self.id);

        self.buffer.reset();

        self.previous_sample = None;
        self.cursor_sample = SampleIndex::default();
        self.cursor_ts = Timestamp::default();
        self.first_visible_sample = None;
        self.first_visible_sample_lock = None;
        self.playback_needs_refresh = false;

        self.tracer.reset_conditions();

        self.req_sample_window = SampleIndexRange::default();
        self.half_req_sample_window = SampleIndexRange::default();
        self.eighth_req_sample_window = SampleIndexRange::default();

        self.sample_step_f = 0f64;
        self.sample_step = SampleIndexRange::default();
    }

    pub fn get_limits_as_ts(&self) -> (Timestamp, Timestamp) {
        (
            self.buffer.lower.get_ts(self.state.sample_duration),
            self.buffer.upper.get_ts(self.state.sample_duration),
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
            self.buffer.lower,
            self.buffer.upper,
            self.buffer.contains_eos,
        );

        if self.state.state == gst::State::Playing {
            // stream is playing => let the cursor jump from current timestamp
            // to the sought timestamp without shifting the waveform if possible

            if let Some(first_visible_sample) = self.first_visible_sample {
                if sought_sample >= first_visible_sample
                    && sought_sample <= first_visible_sample + self.req_sample_window
                    && self.buffer.upper - self.buffer.lower >= self.req_sample_window
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

                if self.buffer.contains_eos && sample >= self.buffer.upper {
                    sample = self.buffer.upper;
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

            if self.cursor_sample >= self.buffer.lower && self.cursor_sample < self.buffer.upper {
                // current sample appears after first buffer sample
                if let Some((first_visible_sample_lock, lock_state)) =
                    self.first_visible_sample_lock
                {
                    // There is a scrolling lock constraint
                    match lock_state {
                        LockState::PlayingFirstHalf => {
                            if self.cursor_sample
                                < first_visible_sample_lock + self.half_req_sample_window
                                && self.cursor_sample >= first_visible_sample_lock
                            {
                                // still in first half => keep first visible sample lock
                                Some(first_visible_sample_lock)
                            } else {
                                // No longer in first half => center cursor
                                self.first_visible_sample_lock = None;
                                Some(
                                    self.buffer
                                        .lower
                                        .max(self.cursor_sample - self.half_req_sample_window),
                                )
                            }
                        }
                        LockState::PlayingSecondHalf => {
                            // Take a margin on the right to handle
                            // a seek forward with the right arrow key
                            if self.cursor_sample
                                > first_visible_sample_lock + self.half_req_sample_window
                                && self.cursor_sample
                                    < first_visible_sample_lock
                                        + self.req_sample_window
                                        + self.eighth_req_sample_window
                            {
                                // still in second half
                                if first_visible_sample_lock + self.req_sample_window
                                    < self.buffer.upper
                                {
                                    // and there is still overhead
                                    // => progressively get cursor back to center
                                    match self.previous_sample {
                                        Some(previous_sample) => {
                                            let previous_offset =
                                                previous_sample - first_visible_sample_lock;
                                            if self.cursor_sample > previous_sample {
                                                let delta_cursor =
                                                    self.cursor_sample - previous_sample;
                                                let next_lower = self.buffer.lower.max(
                                                    self.cursor_sample - previous_offset
                                                        + delta_cursor,
                                                );

                                                self.first_visible_sample_lock = Some((
                                                    next_lower,
                                                    LockState::PlayingSecondHalf,
                                                ));
                                                Some(next_lower)
                                            } else {
                                                // cursor jumped before previous sample (seek)
                                                // render the available range
                                                self.first_visible_sample_lock = None;
                                                self.previous_sample = None;

                                                Some(self.buffer.lower)
                                            }
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
                                    let next_lower = if self.buffer.lower + self.req_sample_window
                                        < self.buffer.upper
                                    {
                                        // buffer window is larger than req_sample_window
                                        // set last buffer to the right
                                        let next_lower = self.buffer.upper - self.req_sample_window;
                                        if !self.buffer.contains_eos {
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
                                        self.buffer.lower
                                    };

                                    Some(next_lower)
                                }
                            } else {
                                // No longer in second half => center cursor
                                self.first_visible_sample_lock = None;
                                Some(
                                    self.buffer
                                        .lower
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
                } else if self.cursor_sample + self.half_req_sample_window <= self.buffer.upper {
                    // cursor_sample fits in the first half of the window with last sample further
                    if self.cursor_sample > self.buffer.lower + self.half_req_sample_window {
                        // cursor_sample can be centered
                        Some(self.cursor_sample - self.half_req_sample_window)
                    } else {
                        // cursor_sample before half of displayable window
                        // set origin to the first sample in the buffer
                        // current sample will be displayed between the origin
                        // and the center
                        Some(self.buffer.lower)
                    }
                } else if self.cursor_sample >= self.buffer.upper {
                    // cursor_sample appears after buffer's last sample
                    debug!(
                        concat!(
                            "{}_update_first_visible_sample ",
                            "cursor_sample {} appears after buffer upper bound {}",
                        ),
                        self.id, self.cursor_sample, self.buffer.upper,
                    );
                    if self.buffer.upper + self.req_sample_window >= self.cursor_sample {
                        // rebase buffer attempting to keep in range
                        // even if samples are not rendered yet
                        if self.cursor_sample > self.buffer.lower + self.req_sample_window {
                            Some(self.cursor_sample - self.req_sample_window)
                        } else {
                            Some(self.buffer.lower)
                        }
                    } else {
                        // cursor no longer in range, 2 cases:
                        // - seeking forward
                        // - zoomed-in too much to keep up with the audio stream
                        None
                    }
                } else if self.buffer.lower + self.req_sample_window < self.buffer.upper {
                    // buffer window is larger than req_sample_window
                    // set last buffer to the right
                    Some(self.buffer.upper - self.req_sample_window)
                } else {
                    // buffer window is smaller than req_sample_window
                    // set first sample to the left
                    Some(self.buffer.lower)
                }
            } else if self.cursor_sample + self.req_sample_window > self.buffer.lower
                && self.cursor_sample < self.buffer.upper
            {
                // cursor is close enough to current buffer
                // => render what can be rendered
                Some(self.buffer.lower)
            } else {
                // cursor_sample out of buffer range
                // => wait until situation clarifies
                debug!(
                    concat!(
                        "{}_update_first_visible_sample cursor_sample {} ",
                        "out of buffer range [{}, {}]",
                    ),
                    self.id, self.cursor_sample, self.buffer.lower, self.buffer.upper,
                );
                None
            }
        } else {
            debug!("{}_update_first_visible_sample buffer not ready", self.id);
            None
        };
    }

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

        let width_changed = match self.tracer.update_width(width) {
            Some(previous_width) => {
                if self.tracer.width != 0 {
                    scale_num = width as usize;
                    scale_denom = previous_width as usize;
                }
                true
            }
            None => false,
        };

        if self.tracer.update_height(height).is_some() {
            self.buffer.update_height(height);

            self.conditions_changed = true;
            self.buffer.force_extraction = true;
        };

        if duration_changed || width_changed {
            self.conditions_changed = true;
            self.buffer.force_extraction = true;

            self.update_sample_window();

            // update first sample in order to match new conditions
            if scale_num != 0 {
                self.first_visible_sample = match self.first_visible_sample {
                    Some(first_visible_sample) => {
                        let new_first_visible_sample = first_visible_sample
                            + self.req_sample_window.get_scaled(scale_num, scale_denom);

                        if new_first_visible_sample > self.sample_step {
                            let lower = self.buffer.lower;
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
                                self.buffer.lower,
                                self.buffer.upper,
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

        self.tracer.update_x_step(self.sample_step_f);
    }

    fn update_sample_window(&mut self) {
        // force sample window to an even number of samples so that the cursor can be centered
        // and make sure to cover at least the requested width
        let half_req_sample_window = (self.sample_step_f * self.tracer.width_f / 2f64) as usize;
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

    pub fn render(&mut self, cr: &cairo::Context) -> Option<WaveformMetrics> {
        let first_visible_sample = self.first_visible_sample?;

        let sample_duration = self.state.sample_duration;

        let first_index =
            (first_visible_sample - self.buffer.lower).get_step_range(self.sample_step);
        let first_ts = first_visible_sample.get_ts(sample_duration);

        assert!(first_visible_sample < self.buffer.upper);
        let (last_index, last) = {
            let visible_sample_range = self.buffer.upper - first_visible_sample;
            if visible_sample_range > self.req_sample_window {
                (
                    self.req_sample_window.get_step_range(self.sample_step) + first_index,
                    SamplePosition {
                        ts: (first_visible_sample + self.req_sample_window)
                            .get_ts(self.state.sample_duration),
                        x: self.tracer.width_f,
                    },
                )
            } else {
                let delta_index = visible_sample_range.get_step_range(self.sample_step);
                (
                    delta_index + first_index,
                    SamplePosition {
                        ts: self.buffer.upper.get_ts(self.state.sample_duration),
                        x: delta_index as f64 * self.tracer.x_step_f,
                    },
                )
            }
        };

        if last_index < first_index + 1 {
            return None;
        }

        self.tracer
            .draw(cr, &self.buffer, first_index, last_index, last.x);

        let cursor = if self.cursor_sample >= first_visible_sample
            && self.cursor_sample <= first_visible_sample + self.req_sample_window
        {
            let delta_index =
                (self.cursor_sample - first_visible_sample).get_step_range(self.sample_step);
            Some(SamplePosition {
                ts: self.cursor_ts,
                x: delta_index as f64 * self.tracer.x_step_f,
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

    fn extract_waveform_samples(&mut self, audio_buffer: &AudioBuffer) {
        let extraction_range = if audio_buffer.upper - audio_buffer.lower <= self.req_sample_window
        {
            // can use the full window
            trace!(
                "{}_extract_waveform_samples using full window, range [{}, {}]",
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
                    "{}_extract_waveform_samples cursor not in the window: first_visible_sample ",
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
                                "{}_extract_waveform_samples cursor in the window: ",
                                "first_visible_sample {}, cursor {}, range [{}, {}]",
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
                                "{}_extract_waveform_samples first_visible_sample ",
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
                                "{}_extract_waveform_samples centering cursor: {}",
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
                                "{}_extract_waveform_samples cursor: {} in second half",
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
                            concat!(
                                "{}_extract_waveform_samples cursor {} in first half ",
                                "or before range [{}, {}]",
                            ),
                            self.id, self.cursor_sample, audio_buffer.lower, audio_buffer.upper,
                        );

                        // use defaults
                        None
                    }
                }
            }
        };

        let (lower, upper) = extraction_range.unwrap_or_else(|| {
            (
                audio_buffer.lower,
                audio_buffer.upper.min(
                    audio_buffer.lower + self.req_sample_window + self.eighth_req_sample_window,
                ),
            )
        });

        self.buffer
            .extract(audio_buffer, lower, upper, self.sample_step);
    }
}

impl SampleExtractor for WaveformRenderer {
    fn get_extraction_state(&self) -> &SampleExtractionState {
        &self.state
    }

    fn get_extraction_state_mut(&mut self) -> &mut SampleExtractionState {
        &mut self.state
    }

    fn get_lower(&self) -> SampleIndex {
        self.first_visible_sample
            .map_or(self.buffer.lower, |sample| {
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
        self.tracer.set_channels(channels);
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

    fn update_concrete_state(&mut self, other: &mut Self) {
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
            self.sample_step = other.sample_step;
            self.sample_step_f = other.sample_step_f;
            self.req_sample_window = other.req_sample_window;
            self.half_req_sample_window = other.half_req_sample_window;
            self.eighth_req_sample_window = other.eighth_req_sample_window;

            self.buffer.update_from_other(&other.buffer);
            self.tracer.update_from_other(&mut other.tracer);

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

        self.playback_needs_refresh = if audio_buffer.contains_eos() && !self.buffer.contains_eos {
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

        self.extract_waveform_samples(audio_buffer);
    }

    fn refresh(&mut self, audio_buffer: &AudioBuffer) {
        if self.is_initialized {
            // Note: current state is up to date (updated from DoubleAudioRenderer)

            self.extract_waveform_samples(audio_buffer);

            self.playback_needs_refresh = {
                if self.playback_needs_refresh && self.buffer.contains_eos {
                    debug!("{}_refresh resetting playback_needs_refresh", self.id);
                }

                !self.buffer.contains_eos
            };
        } else {
            debug!("{}_refresh not initialized yet", self.id);
        }
    }
}
