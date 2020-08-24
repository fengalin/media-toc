use gstreamer as gst;

use log::{debug, trace};

use std::boxed::Box;

use media::{
    sample_extractor::SampleExtractionState, AudioBuffer, AudioChannel, DoubleAudioBuffer,
    SampleExtractor, SampleIndex, SampleIndexRange, Timestamp,
};

use metadata::Duration;

use super::{Image, WaveformImage};

pub struct DoubleWaveformRenderer;

impl DoubleWaveformRenderer {
    pub fn new_dbl_audio_buffer(buffer_duration: Duration) -> DoubleAudioBuffer<WaveformRenderer> {
        DoubleAudioBuffer::new(
            buffer_duration,
            Box::new(WaveformRenderer::new(1)),
            Box::new(WaveformRenderer::new(2)),
        )
    }
}

#[derive(Default)]
pub struct SamplePosition {
    pub x: f64,
    pub ts: Timestamp,
}

#[derive(Default)]
pub struct ImagePositions {
    pub first: SamplePosition,
    pub cursor: Option<SamplePosition>,
    pub last: SamplePosition,
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

// A WaveformRenderer hosts one of the Waveform images of the double buffering
// mechanism based on the SampleExtractor trait.
// It is responsible for preparing an up to date Waveform image which will be
// diplayed upon UI request. Up to date signifies that the Waveform image
// contains all the samples that can fit in the target window at the specified
// resolution for current playback timestamp.
// Whenever possible, the WaveformRenderer attempts to have the Waveform scroll
// between frames with current playback position in the middle so that the
// user can seek forward or backward around current timestamp.
#[derive(Default)]
pub struct WaveformRenderer {
    is_initialized: bool,
    state: SampleExtractionState,
    conditions_changed: bool,

    pub image: WaveformImage,

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
    width_f: f64,
    sample_step_f: f64,
    req_sample_window: SampleIndexRange,
    half_req_sample_window: SampleIndexRange,
    quarter_req_sample_window: SampleIndexRange,
}

impl WaveformRenderer {
    pub fn new(id: usize) -> Self {
        WaveformRenderer {
            image: WaveformImage::new(id),
            ..WaveformRenderer::default()
        }
    }

    pub fn reset(&mut self) {
        debug!("{}_reset", self.image.id);

        self.conditions_changed = false;

        self.reset_sample_conditions();
        self.image.cleanup();

        self.req_duration_per_1000px = Duration::default();
        self.width_f = 0f64;
    }

    fn reset_sample_conditions(&mut self) {
        debug!("{}_reset_sample_conditions", self.image.id);

        self.is_initialized = false;
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
        if !self.is_initialized {
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

    fn refresh_ts(&mut self) {
        match self.first_visible_sample_lock {
            Some((_first_visible_sample_lock, LockState::Seeking)) => (),
            _ => {
                if let Some((ts, mut sample)) = self.get_current_sample() {
                    self.previous_sample = Some(self.cursor_sample);

                    if self.image.contains_eos && sample >= self.image.upper {
                        sample = self.image.upper;
                        sample
                            .try_dec()
                            .expect("adjusting cursor_sample to last sample in stream");
                    }
                    self.cursor_sample = sample;
                    self.cursor_ts = ts;
                }
            }
        }
    }

    // Update to current timestamp and compute the first sample to display.
    fn update_first_visible_sample(&mut self) {
        self.first_visible_sample = if self.image.is_ready {
            self.refresh_ts();

            if self.cursor_sample >= self.image.lower {
                // current sample appears after first sample on image
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
                                    self.image
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
                                        + self.quarter_req_sample_window
                            {
                                // still in second half
                                if first_visible_sample_lock + self.req_sample_window
                                    < self.image.upper
                                {
                                    // and there is still overhead
                                    // => progressively get cursor back to center
                                    match self.previous_sample {
                                        Some(previous_sample) => {
                                            let previous_offset =
                                                previous_sample - first_visible_sample_lock;
                                            match self.cursor_sample.checked_sub(previous_sample) {
                                                Some(delta_cursor) => {
                                                    let next_lower = self.image.lower.max(
                                                        self.cursor_sample - previous_offset
                                                            + delta_cursor,
                                                    );

                                                    self.first_visible_sample_lock = Some((
                                                        next_lower,
                                                        LockState::PlayingSecondHalf,
                                                    ));
                                                    Some(next_lower)
                                                }
                                                None => {
                                                    // cursor jumped before previous sample (seek)
                                                    // render the available range
                                                    self.first_visible_sample_lock = None;
                                                    self.previous_sample = None;

                                                    Some(self.image.lower)
                                                }
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
                                    let next_lower = if self.image.lower + self.req_sample_window
                                        < self.image.upper
                                    {
                                        // image range is larger than req_sample_window
                                        // render the end of the available range
                                        let next_lower = self.image.upper - self.req_sample_window;
                                        if !self.image.contains_eos {
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
                                        // image range is smaller than req_sample_window
                                        // render the available range
                                        self.first_visible_sample_lock = None;
                                        self.image.lower
                                    };

                                    Some(next_lower)
                                }
                            } else {
                                // No longer in second half => center cursor
                                self.first_visible_sample_lock = None;
                                Some(
                                    self.image
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
                } else if self.cursor_sample + self.half_req_sample_window <= self.image.upper {
                    // cursor_sample fits in the first half of the window with last sample further
                    if self.cursor_sample > self.image.lower + self.half_req_sample_window {
                        // cursor_sample can be centered
                        Some(self.cursor_sample - self.half_req_sample_window)
                    } else {
                        // cursor_sample before half of displayable window
                        // set origin to the first sample of the image
                        // cursor sample will be displayed between the origin
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
                    // image range is larger than req_sample_window
                    // render the end of the available samples
                    Some(self.image.upper - self.req_sample_window)
                } else {
                    // image range is smaller than req_sample_window
                    // render the available samples
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
        let (mut scale_num, mut scale_denom) =
            if duration_per_1000px == self.req_duration_per_1000px {
                (0, 0)
            } else {
                let prev_duration = self.req_duration_per_1000px;
                self.req_duration_per_1000px = duration_per_1000px;
                debug!(
                    "{}_update_conditions duration/1000px {} -> {}",
                    self.image.id, prev_duration, self.req_duration_per_1000px,
                );
                self.update_sample_step();
                (duration_per_1000px.as_usize(), prev_duration.as_usize())
            };

        if let Some(prev_width) = self.image.update_width(width) {
            if width != 0 {
                scale_num = prev_width as usize;
                scale_denom = width as usize;

                self.width_f = f64::from(width);
            }
        }

        self.conditions_changed |= self.image.update_height(height).is_some();

        if scale_denom != 0 {
            self.update_sample_window();

            // update first sample in order to match new conditions
            if scale_num != 0 {
                self.first_visible_sample = match self.first_visible_sample {
                    Some(first_visible_sample) => {
                        let new_first_visible_sample = first_visible_sample
                            + self.req_sample_window.get_scaled(scale_num, scale_denom);

                        if new_first_visible_sample > self.image.sample_step {
                            let lower = self.image.lower;
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
        self.conditions_changed = true;

        self.image.update_sample_step(self.sample_step_f);
        self.is_initialized = self.image.sample_step != SampleIndexRange::default();
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
    pub fn get_image(&mut self) -> Option<(&mut Image, ImagePositions)> {
        self.update_first_visible_sample();

        let first_visible_sample = match self.first_visible_sample {
            Some(first_visible_sample) => first_visible_sample,
            None => {
                debug!(
                    "{}_get_image first_visible_sample not available",
                    self.image.id
                );
                return None;
            }
        };

        let sample_duration = self.state.sample_duration;

        let first_index =
            (first_visible_sample - self.image.lower).get_step_range(self.image.sample_step);
        let first = SamplePosition {
            x: first_index as f64 * self.image.x_step_f,
            ts: first_visible_sample.get_ts(sample_duration),
        };

        let cursor = self
            .cursor_sample
            .checked_sub(first_visible_sample)
            .and_then(|range_to_cursor| {
                if range_to_cursor <= self.req_sample_window {
                    let delta_index = range_to_cursor.get_step_range(self.image.sample_step);
                    Some(SamplePosition {
                        x: delta_index as f64 * self.image.x_step_f,
                        ts: self.cursor_ts,
                    })
                } else {
                    None
                }
            });

        let last = {
            let visible_sample_range = self.image.upper.checked_sub(first_visible_sample).unwrap();
            if visible_sample_range > self.req_sample_window {
                SamplePosition {
                    x: self.width_f,
                    ts: (first_visible_sample + self.req_sample_window)
                        .get_ts(self.state.sample_duration),
                }
            } else {
                let delta_index = visible_sample_range.get_step_range(self.image.sample_step);
                SamplePosition {
                    x: delta_index as f64 * self.image.x_step_f,
                    ts: self.image.upper.get_ts(self.state.sample_duration),
                }
            }
        };

        let sample_step = self.image.sample_step_f;

        Some((
            self.image.get_image(),
            ImagePositions {
                first,
                cursor,
                last,
                sample_duration,
                sample_step,
            },
        ))
    }

    fn render(&mut self, audio_buffer: &AudioBuffer) {
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
        } else {
            // not able to merge buffer with current waveform
            // synchronize on latest segment received
            let segment_lower = audio_buffer.segment_lower();
            debug!(
                concat!(
                    "{}_render not able to merge: ",
                    "cursor {}, image [{}, {}], buffer [{}, {}], segment: {}",
                ),
                self.image.id,
                self.cursor_sample,
                self.image.lower,
                self.image.upper,
                audio_buffer.lower,
                audio_buffer.upper,
                segment_lower,
            );

            self.first_visible_sample = None;
            self.first_visible_sample_lock = None;

            (segment_lower, audio_buffer.upper)
        };

        // Second step: find the range to display
        let extraction_range = if upper - lower <= self.req_sample_window {
            // image can use the full window
            trace!(
                "{}_render using full window, range [{}, {}]",
                self.image.id,
                lower,
                upper,
            );

            self.first_visible_sample = None;
            self.first_visible_sample_lock = None;

            Some((lower, upper))
        } else if self.cursor_sample <= lower || self.cursor_sample >= upper {
            trace!(
                concat!(
                    "{}_render cursor not in the window: first_visible_sample ",
                    "{:?}, cursor {}, merged range [{}, {}]",
                ),
                self.image.id,
                self.first_visible_sample,
                self.cursor_sample,
                lower,
                upper,
            );

            self.first_visible_sample = None;
            self.first_visible_sample_lock = None;

            // use defaults
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
                                "{}_render cursor in the window: first_visible_sample ",
                                "{}, cursor {}, merged range [{}, {}]",
                            ),
                            self.image.id,
                            first_visible_sample,
                            self.cursor_sample,
                            lower,
                            upper,
                        );

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
                                "{}_render first_visible_sample ",
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
                                    // use defaults
                                    None
                                }
                            }
                            Some((first_visible_sample, lock_state)) => match lock_state {
                                LockState::PlayingFirstHalf
                                | LockState::PlayingSecondHalf
                                | LockState::Seeking => {
                                    self.first_visible_sample = None;
                                    None
                                }
                                LockState::PlayingRange | LockState::RestoringInitialPos => {
                                    // In range playing, the cursor may shortly exit the window
                                    // at the end of the range
                                    // but we still want to keep the waveform locked
                                    self.first_visible_sample_lock =
                                        Some((first_visible_sample, lock_state));

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
                                "{}_render centering cursor: {}",
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
                                "{}_render cursor: {} in second half",
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
                            "{}_render cursor {} in first half or before range [{}, {}]",
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

        let (lower, upper) = extraction_range.unwrap_or_else(|| {
            (
                audio_buffer.lower,
                audio_buffer
                    .upper
                    .min(audio_buffer.lower + self.req_sample_window + self.half_req_sample_window),
            )
        });

        self.image.render(audio_buffer, lower, upper);
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

    fn update_concrete_state(&mut self, other: &mut WaveformRenderer) {
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
            self.is_initialized = other.is_initialized;
            self.req_duration_per_1000px = other.req_duration_per_1000px;
            self.width_f = other.width_f;
            self.sample_step_f = other.sample_step_f;
            self.req_sample_window = other.req_sample_window;
            self.half_req_sample_window = other.half_req_sample_window;
            self.quarter_req_sample_window = other.quarter_req_sample_window;

            other.conditions_changed = false;
        } // else: other has nothing new

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

        self.render(audio_buffer);

        self.playback_needs_refresh = if audio_buffer.contains_eos() && !self.image.contains_eos {
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

            self.render(audio_buffer);

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
