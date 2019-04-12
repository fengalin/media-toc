use gstreamer as gst;

use log::{debug, trace};

use std::{
    any::Any,
    boxed::Box,
    sync::{Arc, Mutex},
};

use crate::media::{
    sample_extractor::SampleExtractionState, AudioBuffer, AudioChannel, DoubleAudioBuffer,
    Duration, SampleExtractor, SampleIndex, SampleIndexRange, Timestamp,
};

use super::{Image, WaveformImage};

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

pub struct SamplePosition {
    pub x: f64,
    pub ts: Timestamp,
}

pub struct ImagePositions {
    pub first: SamplePosition,
    pub last: Option<SamplePosition>,
    pub current_x: Option<f64>,
    pub sample_duration: Duration,
    pub sample_step: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LockState {
    Playing,
    PlayingRange,
    RestoringInitialPos,
    Seeking,
}

// A WaveformBuffer hosts one of the two buffers of the double buffering
// mechanism based on the SampleExtractor trait.
// It is responsible for preparing an up to date Waveform image which will be
// diplayed upon UI request. Up to date signifies that the Waveform image
// contains all the samples that can fit in the target window at the specified
// resolution for current playback timestamp.
// Whenever possible, the WaveformBuffer attempts to have the Waveform scroll
// between frames with current playback position in the middle so that the
// user can seek forward or backward around current timestamp.
pub struct WaveformBuffer {
    state: SampleExtractionState,
    conditions_changed: bool,

    image: WaveformImage,

    previous_sample: Option<SampleIndex>,
    cursor_sample: SampleIndex, // The sample idx currently under the cursor (might be different from
    // current sample idx during seeks)
    cursor_ts: Timestamp,
    first_visible_sample: Option<SampleIndex>,
    first_visible_sample_lock: Option<(i64, LockState)>, // FIXME: switch to SampleIndex instead of i64

    // During playback, we take advantage of the running time and thus
    // the stream of incoming samples to refresh the waveform.
    // When EOS is reached, no more samples are received, so refresh
    // must be forced in order to compute the samples window to render
    pub playback_needs_refresh: bool,

    req_duration_per_1000px: Duration,
    width: i32,
    width_f: f64,
    sample_step_f: f64,
    req_sample_window: SampleIndexRange,
    half_req_sample_window: SampleIndexRange,
    quarter_req_sample_window: SampleIndexRange,
}

impl WaveformBuffer {
    pub fn new(id: usize) -> Self {
        WaveformBuffer {
            state: SampleExtractionState::new(),
            conditions_changed: false,

            image: WaveformImage::new(id),

            previous_sample: None,
            cursor_sample: SampleIndex::default(),
            cursor_ts: Timestamp::default(),
            first_visible_sample: None,
            first_visible_sample_lock: None,
            playback_needs_refresh: false,

            req_duration_per_1000px: Duration::default(),
            width: 0,
            width_f: 0f64,
            sample_step_f: 0f64,
            req_sample_window: SampleIndexRange::default(),
            half_req_sample_window: SampleIndexRange::default(),
            quarter_req_sample_window: SampleIndexRange::default(),
        }
    }

    fn reset(&mut self) {
        debug!("{}_reset", self.image.id);

        self.conditions_changed = false;

        self.reset_sample_conditions();
        self.image.cleanup();

        self.req_duration_per_1000px = Duration::default();
        self.width = 0;
        self.width_f = 0f64;
    }

    fn reset_sample_conditions(&mut self) {
        debug!("{}_reset_sample_conditions", self.image.id);
        self.previous_sample = None;
        self.cursor_sample = SampleIndex::default();
        self.cursor_ts = Timestamp::default();
        self.first_visible_sample = None;
        self.first_visible_sample_lock = None;
        self.playback_needs_refresh = false;

        self.sample_step_f = 0f64;
        self.req_sample_window = SampleIndexRange::default();
        self.half_req_sample_window = SampleIndexRange::default();
        self.quarter_req_sample_window = SampleIndexRange::default();

        self.image.cleanup_sample_conditions();
    }

    pub fn get_limits_as_ts(&self) -> (Timestamp, Timestamp) {
        (
            self.image.lower.get_ts(self.state.sample_duration),
            self.image.upper.get_ts(self.state.sample_duration),
        )
    }

    pub fn get_half_window_duration(&self) -> Duration {
        self.half_req_sample_window
            .get_duration(self.state.sample_duration)
    }

    pub fn seek(&mut self, target: Timestamp) {
        if self.image.sample_step == SampleIndexRange::default() {
            return;
        }

        let sought_sample = SampleIndex::from_ts(target, self.state.sample_duration)
            .get_aligned(self.image.sample_step);

        debug!(
            concat!(
                "{}_seek cursor_sample {}, sought sample {} ({}), ",
                "state: {:?}, image [{}, {}], contains_eos: {}",
            ),
            self.image.id,
            self.cursor_sample,
            sought_sample,
            target,
            self.state.state,
            self.image.lower,
            self.image.upper,
            self.image.contains_eos,
        );

        if self.state.state == gst::State::Playing {
            // stream is playing => let the cursor jump from current timestamp
            // to the sought timestamp without shifting the waveform if possible

            if let Some(first_visible_sample) = self.first_visible_sample {
                if sought_sample >= first_visible_sample
                    && sought_sample <= first_visible_sample + self.req_sample_window
                    && self.image.upper - self.image.lower >= self.req_sample_window
                {
                    // sought sample is in current window
                    // and the window is large enough for a constraint
                    // => lock the first sample so that the cursor appears
                    // at the sought idx without abrutely scrolling
                    // the waveform.
                    self.first_visible_sample_lock =
                        Some((first_visible_sample.as_i64(), LockState::Seeking));
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
                    Some((first_visible_sample as usize).into())
                }
                _ => None,
            };
        }

        self.previous_sample = None;
        self.cursor_sample = sought_sample;
    }

    pub fn start_play_range(&mut self) {
        if self.image.sample_step == SampleIndexRange::default() {
            return;
        }

        if let Some(first_visible_sample) = self.first_visible_sample {
            self.first_visible_sample_lock =
                Some((first_visible_sample.as_i64(), LockState::PlayingRange));
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

                if self.image.contains_eos && sample >= self.image.upper {
                    sample = self.image.upper;
                    sample.dec();
                }
                self.cursor_sample = sample;
                self.cursor_ts = ts;
            }
        }
    }

    // Update to current timestamp and compute the first sample to display.
    fn update_first_visible_sample(&mut self, last_frame_ts: Timestamp, next_frame_ts: Timestamp) {
        self.first_visible_sample = if self.image.is_ready {
            self.refresh_ts(last_frame_ts, next_frame_ts);

            if self.cursor_sample >= self.image.lower {
                // current sample appears after first buffer sample
                if let Some((first_visible_sample_lock, lock_state)) =
                    self.first_visible_sample_lock
                {
                    // There is a scrolling lock constraint

                    match lock_state {
                        LockState::Playing => {
                            let offset_to_center = self.cursor_sample.as_i64()
                                - self.half_req_sample_window.as_i64()
                                - first_visible_sample_lock;

                            let threshold = 2 * self.image.sample_step.as_i64();
                            if offset_to_center < -threshold {
                                // cursor in first half of the window
                                // keep origin on the first sample upon seek
                                self.first_visible_sample_lock =
                                    Some((first_visible_sample_lock, lock_state));
                                Some((first_visible_sample_lock as usize).into())
                            } else if offset_to_center <= threshold {
                                // reached the center => keep cursor there
                                self.first_visible_sample_lock = None;
                                Some(
                                    self.image
                                        .lower
                                        .max(self.cursor_sample - self.half_req_sample_window),
                                )
                            } else if SampleIndex::new(first_visible_sample_lock as usize)
                                + self.req_sample_window
                                < self.image.upper
                            {
                                match self.previous_sample {
                                    Some(previous_sample) => {
                                        // Cursor is on the right half of the window
                                        // and the target sample window doesn't exceed the end
                                        // of the rendered waveform yet
                                        // => progressively get cursor back to center
                                        let previous_offset =
                                            previous_sample.as_i64() - first_visible_sample_lock;
                                        let delta_cursor =
                                            self.cursor_sample.as_i64() - previous_sample.as_i64();
                                        let next_lower = (self.image.lower.as_i64()).max(
                                            self.cursor_sample.as_i64() - previous_offset
                                                + delta_cursor,
                                        );

                                        self.first_visible_sample_lock =
                                            Some((next_lower, LockState::Playing));
                                        Some((next_lower as usize).into())
                                    }
                                    None => return,
                                }
                            } else {
                                // Not enough overhead to get cursor back to center
                                // Follow toward the last sample
                                let next_lower = if self.image.lower + self.req_sample_window
                                    < self.image.upper
                                {
                                    // buffer window is larger than req_sample_window
                                    // set last buffer to the right
                                    let next_lower = self.image.upper - self.req_sample_window;
                                    if !self.image.contains_eos {
                                        // but keep the constraint in case more samples
                                        // are added afterward
                                        self.first_visible_sample_lock =
                                            Some((next_lower.as_i64(), LockState::Playing));
                                    } else {
                                        // reached EOS => don't expect returning to center
                                        self.first_visible_sample_lock = None;
                                    }

                                    next_lower
                                } else {
                                    // buffer window is smaller than req_sample_window
                                    // set first sample to the left
                                    self.first_visible_sample_lock = None;
                                    self.image.lower
                                };

                                Some(next_lower)
                            }
                        }
                        LockState::PlayingRange => {
                            // keep origin on the first sample upon seek
                            Some((first_visible_sample_lock as usize).into())
                        }
                        LockState::RestoringInitialPos => {
                            self.first_visible_sample_lock = None;
                            Some((first_visible_sample_lock as usize).into())
                        }
                        LockState::Seeking => {
                            // Wait until new sample delta can be computed
                            return;
                        }
                    }
                } else if self.cursor_sample + self.half_req_sample_window <= self.image.upper {
                    // cursor_sample fits in the first half of the window with last sample further
                    if self.cursor_sample > self.image.lower + self.half_req_sample_window {
                        // cursor_sample can be centered
                        Some(self.cursor_sample - self.half_req_sample_window)
                    } else {
                        // cursor_sample before half of displayable window
                        // set origin to the first sample in the buffer
                        // current sample will be displayed between the origin
                        // and the center
                        Some(self.image.lower)
                    }
                } else if self.cursor_sample >= self.image.upper {
                    // cursor_sample appears after image last sample
                    debug!(
                        concat!(
                            "{}_update_first_visible_sample ",
                            "cursor_sample {} appears above image upper bound {}",
                        ),
                        self.image.id, self.cursor_sample, self.image.upper,
                    );
                    if self.image.upper + self.req_sample_window >= self.cursor_sample {
                        // rebase image attempting to keep in range
                        // even if samples are not rendered yet
                        if self.cursor_sample > self.image.lower + self.req_sample_window {
                            Some(self.cursor_sample - self.req_sample_window)
                        } else {
                            Some(self.image.lower)
                        }
                    } else {
                        // cursor no longer in range, 2 cases:
                        // - seeking forward
                        // - zoomed-in too much to keep up with the audio stream
                        None
                    }
                } else if self.image.lower + self.req_sample_window < self.image.upper {
                    // buffer window is larger than req_sample_window
                    // set last buffer to the right
                    Some(self.image.upper - self.req_sample_window)
                } else {
                    // buffer window is smaller than req_sample_window
                    // set first sample to the left
                    Some(self.image.lower)
                }
            } else if self.cursor_sample + self.req_sample_window > self.image.lower {
                // cursor is close enough to the image
                // => render what can be rendered
                Some(self.image.lower)
            } else {
                // cursor_sample appears before image first sample
                // => wait until situation clarifies
                debug!(
                    concat!(
                        "{}_update_first_visible_sample cursor_sample {} ",
                        "appears before image first sample {}",
                    ),
                    self.image.id, self.cursor_sample, self.image.lower
                );
                None
            }
        } else {
            debug!(
                "{}_update_first_visible_sample image not ready",
                self.image.id
            );
            None
        };
    }

    // Update rendering conditions
    pub fn update_conditions(&mut self, duration_per_1000px: Duration, width: i32, height: i32) {
        let (duration_changed, mut scale_factor) =
            if duration_per_1000px == self.req_duration_per_1000px {
                (false, 0f64)
            } else {
                let prev_duration = self.req_duration_per_1000px;
                self.req_duration_per_1000px = duration_per_1000px;
                debug!(
                    "{}_update_conditions duration/1000px {} -> {}",
                    self.image.id, prev_duration, self.req_duration_per_1000px,
                );
                self.update_sample_step();
                (true, duration_per_1000px.as_f64() / prev_duration.as_f64())
            };

        let width_changed = if width == self.width {
            false
        } else {
            let width_f = f64::from(width);
            let prev_width_f = self.width_f;

            if prev_width_f != 0f64 {
                scale_factor = width_f / prev_width_f;
            }

            debug!(
                "{}_update_conditions width {} -> {} ({})",
                self.image.id, self.width, width, scale_factor,
            );

            self.width = width;
            self.width_f = width_f;
            true
        };

        self.image.update_dimensions(width, height);

        if duration_changed || width_changed {
            self.update_sample_window();

            // update first sample in order to match new conditions
            if scale_factor != 0f64 {
                self.first_visible_sample = match self.first_visible_sample {
                    Some(first_visible_sample) => {
                        let new_first_visible_sample = first_visible_sample.as_i64()
                            + ((self.cursor_sample.as_i64() - first_visible_sample.as_i64()) as f64
                                * (1f64 - scale_factor)) as i64;

                        if new_first_visible_sample > self.image.sample_step.as_i64() {
                            let lower = self.image.lower.as_i64();
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
                                self.image.id,
                                self.image.lower,
                                self.image.upper,
                                self.req_sample_window,
                                first_visible_sample,
                                new_first_visible_sample,
                                self.image.sample_step,
                                self.cursor_sample,
                            );

                            if let Some((_first_visible_sample_lock, lock_state)) =
                                self.first_visible_sample_lock.take()
                            {
                                // There is a first visible sample constraint
                                // => adapt it to match the new zoom
                                self.first_visible_sample_lock =
                                    Some((new_first_visible_sample, lock_state));
                            }

                            Some((new_first_visible_sample as usize).into())
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
        self.conditions_changed = true;

        self.image.update_sample_step(self.sample_step_f);
    }

    fn update_sample_window(&mut self) {
        // force sample window to an even number of samples so that the cursor can be centered
        // and make sure to cover at least the requested width
        let half_req_sample_window = (self.sample_step_f * self.width_f / 2f64) as usize;
        let req_sample_window = half_req_sample_window * 2;

        if req_sample_window != self.req_sample_window.as_usize() {
            debug!(
                "{}_update_sample_window smpl.window prev. {}, new {}",
                self.image.id, self.req_sample_window, req_sample_window
            );
        }

        self.req_sample_window = req_sample_window.into();
        self.half_req_sample_window = half_req_sample_window.into();
        self.quarter_req_sample_window = (half_req_sample_window / 2).into();
        self.conditions_changed = true;
    }

    // Get the waveform as an image in current conditions.
    pub fn get_image(
        &mut self,
        last_frame_ts: Timestamp,
        next_frame_ts: Timestamp,
    ) -> (Timestamp, Option<(&mut Image, ImagePositions)>) {
        self.update_first_visible_sample(last_frame_ts, next_frame_ts);
        match self.first_visible_sample {
            Some(first_visible_sample) => {
                let current_x = if self.cursor_sample >= first_visible_sample
                    && self.cursor_sample <= first_visible_sample + self.req_sample_window
                {
                    Some(
                        (self.cursor_sample - first_visible_sample).as_f64()
                            / self.image.sample_step_f,
                    )
                } else {
                    None
                };

                let x_offset = ((first_visible_sample - self.image.lower)
                    .get_step_range(self.image.sample_step)
                    * self.image.x_step) as f64;

                let last_opt = match self.image.last {
                    Some(ref last) => {
                        let delta_x = last.x - x_offset;

                        let last_x = delta_x.min(self.width_f);
                        if last_x.is_sign_positive() {
                            Some(SamplePosition {
                                x: last_x,
                                ts: (first_visible_sample
                                    + SampleIndexRange::new(
                                        (last_x * self.image.sample_step_f) as usize,
                                    ))
                                .get_ts(self.state.sample_duration),
                            })
                        } else {
                            None
                        }
                    }
                    None => None,
                };

                let sample_duration = self.state.sample_duration;
                let sample_step = self.image.sample_step_f;

                (
                    self.cursor_ts,
                    Some((
                        self.image.get_image(),
                        ImagePositions {
                            first: SamplePosition {
                                x: x_offset,
                                ts: first_visible_sample.get_ts(sample_duration),
                            },
                            last: last_opt,
                            current_x,
                            sample_duration,
                            sample_step,
                        },
                    )),
                )
            }
            None => (self.cursor_ts, None),
        }
    }

    fn get_sample_range(&mut self, audio_buffer: &AudioBuffer) -> (SampleIndex, SampleIndex) {
        // First step: see how current waveform and the audio_buffer can merge
        let (lower, upper) = if audio_buffer.lower <= self.image.lower
            && audio_buffer.upper >= self.image.upper
        {
            // waveform contained in buffer => regular case
            (audio_buffer.lower, audio_buffer.upper)
        } else if audio_buffer.lower >= self.image.lower && audio_buffer.lower < self.image.upper {
            // last segment further than current image origin
            // but buffer can be merged with current waveform
            // or is contained in current waveform
            (self.image.lower, audio_buffer.upper.max(self.image.upper))
        } else if audio_buffer.lower < self.image.lower && audio_buffer.upper >= self.image.lower {
            // current waveform overlaps with buffer on its left
            // or is contained in buffer
            (audio_buffer.lower, audio_buffer.upper.max(self.image.upper))
        } else {
            // not able to merge buffer with current waveform
            // synchronize on latest segment received
            debug!(
                concat!(
                    "{}_get_sample_range not able to merge: ",
                    "cursor {}, image [{}, {}], buffer [{}, {}], segment: {}",
                ),
                self.image.id,
                self.cursor_sample,
                self.image.lower,
                self.image.upper,
                audio_buffer.lower,
                audio_buffer.upper,
                audio_buffer.segment_lower,
            );

            self.first_visible_sample = None;
            self.first_visible_sample_lock = None;

            (audio_buffer.segment_lower, audio_buffer.upper)
        };

        // Second step: find the range to display
        let extraction_range = if upper - lower <= self.req_sample_window {
            // image can use the full window
            trace!(
                "{}_get_sample_range using full window, range [{}, {}]",
                self.image.id,
                lower,
                upper,
            );

            self.first_visible_sample = None;
            self.first_visible_sample_lock = None;

            Some((lower, upper))
        } else {
            match self.first_visible_sample {
                Some(first_visible_sample) => {
                    if self.cursor_sample >= first_visible_sample
                        && self.cursor_sample < first_visible_sample + self.req_sample_window
                    {
                        // cursor is still in the window => keep it
                        Some((
                            first_visible_sample,
                            upper.min(
                                first_visible_sample
                                    + self.req_sample_window
                                    + self.half_req_sample_window,
                            ),
                        ))
                    } else {
                        debug!(
                            concat!(
                                "{}_get_sample_range first_visible_sample ",
                                "{} and cursor {} not in the same range [{}, {}]",
                            ),
                            self.image.id, first_visible_sample, self.cursor_sample, lower, upper,
                        );

                        match self.first_visible_sample_lock.take() {
                            None => {
                                if self.playback_needs_refresh {
                                    // refresh to the full available range
                                    Some((first_visible_sample, upper))
                                } else {
                                    self.first_visible_sample = None;
                                    None
                                }
                            }
                            Some((first_visible_sample, lock_state)) => match lock_state {
                                LockState::Playing | LockState::Seeking => {
                                    self.first_visible_sample = None;
                                    None
                                }
                                LockState::PlayingRange | LockState::RestoringInitialPos => {
                                    // keep position
                                    self.first_visible_sample_lock =
                                        Some((first_visible_sample, lock_state));

                                    let first_visible_sample =
                                        SampleIndex::new(first_visible_sample as usize);
                                    Some((
                                        first_visible_sample,
                                        upper.min(
                                            first_visible_sample
                                                + self.req_sample_window
                                                + self.half_req_sample_window,
                                        ),
                                    ))
                                }
                            },
                        }
                    }
                }
                None => {
                    if self.cursor_sample > lower + self.half_req_sample_window
                        && self.cursor_sample < upper
                    {
                        // cursor can be centered or is in second half of the window
                        if self.cursor_sample + self.half_req_sample_window < upper {
                            // cursor can be centered
                            trace!(
                                "{}_get_sample_range centering cursor: {}",
                                self.image.id,
                                self.cursor_sample,
                            );

                            Some((
                                self.cursor_sample - self.half_req_sample_window,
                                upper.min(self.cursor_sample + self.req_sample_window),
                            ))
                        } else {
                            // cursor in second half
                            trace!(
                                "{}_get_sample_range cursor: {} in second half",
                                self.image.id,
                                self.cursor_sample,
                            );

                            // attempt to get an optimal range
                            if upper
                                > lower + self.req_sample_window + self.quarter_req_sample_window
                            {
                                Some((
                                    upper - self.req_sample_window - self.quarter_req_sample_window,
                                    upper,
                                ))
                            } else if upper > lower + self.req_sample_window {
                                Some((upper - self.req_sample_window, upper))
                            } else {
                                // use defaults
                                None
                            }
                        }
                    } else {
                        trace!(
                            "{}_get_sample_range cursor {} in first half or before range [{}, {}]",
                            self.image.id,
                            self.cursor_sample,
                            lower,
                            upper,
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
                audio_buffer
                    .upper
                    .min(audio_buffer.lower + self.req_sample_window + self.half_req_sample_window),
            )
        })
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
        self.first_visible_sample
            .map_or(self.image.lower, |sample| {
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
        debug!("{}_cleanup", self.image.id);

        self.state.cleanup();
        self.reset();
    }

    fn seek_complete(&mut self) {
        self.previous_sample = None;

        if let Some((first_visible_sample_lock, LockState::Seeking)) =
            self.first_visible_sample_lock
        {
            self.first_visible_sample_lock = Some((first_visible_sample_lock, LockState::Playing));
        }
    }

    fn set_sample_duration(&mut self, per_sample: Duration, per_1000_samples: Duration) {
        self.reset_sample_conditions();

        debug!(
            "{}_set_sample_duration per_sample {}",
            self.image.id, per_sample
        );
        self.state.sample_duration = per_sample;
        self.state.duration_per_1000_samples = per_1000_samples;
        self.update_sample_step();
        self.update_sample_window();
    }

    fn set_channels(&mut self, channels: &[AudioChannel]) {
        self.image.set_channels(channels);
    }

    fn switch_to_paused(&mut self) {
        match self.first_visible_sample_lock.take() {
            None => self.first_visible_sample = None,
            Some((first_visible_sample, lock_state)) => match lock_state {
                LockState::Playing | LockState::Seeking => self.first_visible_sample = None,
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
            debug!("{}_update_concrete_state conditions_changed", self.image.id);
            self.req_duration_per_1000px = other.req_duration_per_1000px;
            self.width = other.width;
            self.width_f = other.width_f;
            self.sample_step_f = other.sample_step_f;
            self.req_sample_window = other.req_sample_window;
            self.half_req_sample_window = other.half_req_sample_window;
            self.quarter_req_sample_window = other.quarter_req_sample_window;

            other.conditions_changed = false;
        } // else: other has nothing new

        self.state.base_ts = other.state.base_ts;
        self.state.last_ts = other.state.last_ts;
        self.state.is_stable = other.state.is_stable;

        self.image.update_from_other(&mut other.image);
    }

    // This is the entry point for the waveform update.
    // This function tries to merge the samples added to the AudioBuffer
    // since last extraction and adapts to the evolving conditions of
    // the playback position and target rendering dimensions and
    // resolution.
    fn extract_samples(&mut self, audio_buffer: &AudioBuffer) {
        if self.req_sample_window == SampleIndexRange::default() {
            // conditions not defined yet
            return;
        }

        // Get the available sample range considering both
        // the waveform image and the AudioBuffer
        let (lower, upper) = self.get_sample_range(audio_buffer);
        self.image.render(audio_buffer, lower, upper);

        self.playback_needs_refresh = if audio_buffer.eos && !self.image.contains_eos {
            // there won't be any refresh on behalf of audio_buffer
            // and image will still need more sample if playback continues
            debug!(
                "{}_extract_samples setting playback_needs_refresh",
                self.image.id
            );

            true
        } else {
            if self.playback_needs_refresh {
                debug!(
                    "{}_extract_samples resetting playback_needs_refresh",
                    self.image.id
                );
            }
            false
        };
    }

    fn refresh(&mut self, audio_buffer: &AudioBuffer) {
        if self.image.is_initialized {
            // Note: current state is up to date (updated from DoubleAudioBuffer)

            let (lower, upper) = self.get_sample_range(audio_buffer);
            self.image.render(audio_buffer, lower, upper);

            self.playback_needs_refresh = {
                if self.playback_needs_refresh && self.image.contains_eos {
                    debug!("{}_refresh resetting playback_needs_refresh", self.image.id);
                }

                !self.image.contains_eos
            };
        } else {
            debug!("{}_refresh image not ready", self.image.id);
        }
    }
}
