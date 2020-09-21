use log::{debug, trace};

use std::{
    boxed::Box,
    sync::{Arc, RwLock},
};

use media::{
    sample_extractor::SampleExtractionState, AudioBuffer, AudioChannel, DoubleAudioBuffer,
    SampleExtractor, SampleIndex, SampleIndexRange, Timestamp,
};

use metadata::Duration;

use super::{Image, WaveformImage};

pub struct DoubleWaveformRenderer;

impl DoubleWaveformRenderer {
    pub fn new_dbl_audio_buffer(buffer_duration: Duration) -> DoubleAudioBuffer<WaveformRenderer> {
        let shared_state = Arc::new(RwLock::new(SharedState::default()));
        let dimensions = Arc::new(RwLock::new(Dimensions::default()));

        DoubleAudioBuffer::new(
            buffer_duration,
            Box::new(WaveformRenderer::new(
                1,
                Arc::clone(&shared_state),
                Arc::clone(&dimensions),
            )),
            Box::new(WaveformRenderer::new(2, shared_state, dimensions)),
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

#[derive(Clone, Copy, Debug)]
pub enum CursorDirection {
    Backward(SampleIndex),
    Forward,
}

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Frozen,
    Scrollable(CursorDirection),
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Frozen
    }
}

#[derive(Debug, Default)]
pub struct SharedState {
    mode: Mode,
    hold_rendering: bool,

    pub cursor_sample: SampleIndex, // The sample idx currently under the cursor (might be different from
    // current sample idx during seeks)
    cursor_ts: Timestamp,
    pub first_visible_sample: Option<SampleIndex>,

    // During playback, we take advantage of the running time and thus
    // the stream of incoming samples to refresh the waveform.
    // When EOS is reached, no more samples are received, so refresh
    // must be forced in order to compute the samples window to render
    // FIXME rename to something related to scrollable / add to the Scrollable enum?
    pub playback_needs_refresh: bool,
}

impl SharedState {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Default)]
pub struct Dimensions {
    is_initialized: bool,
    req_duration_per_1000px: Duration,
    width_f: f64,
    sample_step_f: f64,
    req_sample_window: SampleIndexRange,
    half_req_sample_window: SampleIndexRange,
    quarter_req_sample_window: SampleIndexRange,
}

impl Dimensions {
    fn reset(&mut self) {
        *self = Self::default();
    }
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
    state: SampleExtractionState,
    pub image: WaveformImage,

    // FIXME maybe a Mutex would be more appropriate
    // check if most contentions happen in write mode
    shared_state: Arc<RwLock<SharedState>>,
    dimensions: Arc<RwLock<Dimensions>>,
}

impl WaveformRenderer {
    pub fn new(
        id: usize,
        shared_state: Arc<RwLock<SharedState>>,
        dimensions: Arc<RwLock<Dimensions>>,
    ) -> Self {
        WaveformRenderer {
            image: WaveformImage::new(id),
            shared_state,
            dimensions,
            ..WaveformRenderer::default()
        }
    }

    pub fn reset(&mut self) {
        debug!("{}_reset", self.image.id);

        self.shared_state.write().unwrap().reset();
        self.dimensions.write().unwrap().reset();
        self.image.cleanup();
    }

    fn reset_sample_conditions(dim: &mut Dimensions, image: &mut WaveformImage) {
        debug!("{}_reset_sample_conditions", image.id);

        dim.reset();
        image.cleanup_sample_conditions();
    }

    pub fn limits_as_ts(&self) -> (Timestamp, Timestamp) {
        (
            self.image.lower.as_ts(self.state.sample_duration),
            self.image.upper.as_ts(self.state.sample_duration),
        )
    }

    pub fn half_window_duration(&self) -> Duration {
        self.dimensions
            .read()
            .unwrap()
            .half_req_sample_window
            .duration(self.state.sample_duration)
    }

    pub fn playback_needs_refresh(&self) -> bool {
        self.shared_state.read().unwrap().playback_needs_refresh
    }

    pub fn freeze(&mut self) {
        self.shared_state.write().unwrap().mode = Mode::Frozen;
    }

    pub fn release(&mut self) {
        self.shared_state.write().unwrap().mode = Mode::Scrollable(CursorDirection::Forward);
    }

    pub fn seek(&mut self) {
        // FIXME see if we could keep the image displayed until seek_done
        // e.g. by setting cursor_sample to None
        // This might also require to be taken care of in render
    }

    /// Adapts to a discontinuation in the audio stream.
    pub fn seek_done(&mut self) {
        let mut shared_state = self.shared_state.write().unwrap();
        shared_state.hold_rendering = false;

        self.refresh_cursor_priv(&mut shared_state);

        let first_visible_sample = match shared_state.first_visible_sample {
            Some(first_visible_sample) => first_visible_sample,
            None => return,
        };

        let (req_sample_window, half_req_sample_window) = {
            let dim = self.dimensions.read().unwrap();
            (dim.req_sample_window, dim.half_req_sample_window)
        };

        let upper_visible_sample = self
            .image
            .upper
            .min(first_visible_sample + req_sample_window);

        if shared_state.cursor_sample >= upper_visible_sample
            || shared_state.cursor_sample < first_visible_sample
        {
            // Cursor no longer in current window => recompute
            shared_state.first_visible_sample = None;
            return;
        }

        // cursor still in current window
        match shared_state.mode {
            Mode::Scrollable(_) => {
                if shared_state.cursor_sample > first_visible_sample + half_req_sample_window {
                    shared_state.mode =
                        Mode::Scrollable(CursorDirection::Backward(shared_state.cursor_sample));
                }
            }
            Mode::Frozen => {
                // Attempt to center cursor
                shared_state.first_visible_sample = None;
                return;
            }
        }
    }

    pub fn refresh_cursor(&mut self) {
        let mut shared_state = self.shared_state.write().unwrap();
        self.refresh_cursor_priv(&mut shared_state);
    }

    #[inline]
    fn refresh_cursor_priv(&self, shared_state: &mut SharedState) {
        if let Some((ts, mut sample)) = self.current_sample() {
            if self.image.contains_eos && sample >= self.image.upper {
                sample = self.image.upper;
                sample
                    .try_dec()
                    .expect("adjusting cursor_sample to last sample in stream");
            }
            shared_state.cursor_sample = sample;
            shared_state.cursor_ts = ts;
        }
    }

    // Computes the first sample to display depending on current mode.
    pub fn refresh_window(&mut self) {
        let mut shared_state = self.shared_state.write().unwrap();

        if !self.image.is_ready {
            debug!(
                "{}_update_first_visible_sample image not ready",
                self.image.id
            );

            shared_state.first_visible_sample = None;
            return;
        }

        let (req_sample_window, half_req_sample_window) = {
            let dim = self.dimensions.read().unwrap();
            (dim.req_sample_window, dim.half_req_sample_window)
        };

        if shared_state.cursor_sample < self.image.lower {
            // cursor appears before image range
            if shared_state.cursor_sample + req_sample_window > self.image.lower {
                // cursor is close enough to the image
                // => render what can be rendered
                debug!(
                    concat!(
                        "{}_refresh_window cursor_sample {} ",
                        "close to image first sample {}",
                    ),
                    self.image.id, shared_state.cursor_sample, self.image.lower
                );

                shared_state.first_visible_sample = Some(self.image.lower);
            } else {
                // cursor_sample appears too far from image first sample
                // => wait until situation clarifies
                debug!(
                    concat!(
                        "{}_refresh_window cursor_sample {} ",
                        "appears before image first sample {}",
                    ),
                    self.image.id, shared_state.cursor_sample, self.image.lower
                );

                shared_state.first_visible_sample = None;
            }

            return;
        }

        // current sample appears after first sample on image

        if shared_state.cursor_sample >= self.image.upper {
            // cursor_sample appears after image last sample
            debug!(
                concat!(
                    "{}_refresh_window ",
                    "cursor_sample {} appears above image upper bound {}",
                ),
                self.image.id, shared_state.cursor_sample, self.image.upper,
            );

            if shared_state.cursor_sample <= self.image.lower + req_sample_window {
                // rebase image attempting to keep in range
                // even if samples are not rendered yet

                if self.image.upper > self.image.lower + req_sample_window {
                    shared_state.first_visible_sample =
                        Some(shared_state.cursor_sample - req_sample_window);
                } else {
                    shared_state.first_visible_sample = Some(self.image.lower);
                }

                return;
            } else {
                // cursor no longer in range, 2 cases:
                // - seeking forward
                // - zoomed-in too much to keep up with the audio stream

                shared_state.first_visible_sample = None;
                return;
            }
        }

        // current sample appears within the image range

        let first_visible_sample = match shared_state.first_visible_sample {
            Some(first_visible_sample) => first_visible_sample,
            None => {
                debug!("{}_refresh_window init first_visible_sample", self.image.id);

                if shared_state.cursor_sample + half_req_sample_window <= self.image.upper {
                    // cursor_sample fits in the first half of the window with last sample further
                    if shared_state.cursor_sample > self.image.lower + half_req_sample_window {
                        // cursor_sample can be centered
                        shared_state.first_visible_sample =
                            Some(shared_state.cursor_sample - half_req_sample_window);
                    } else {
                        // cursor_sample before half of displayable window
                        // set origin to the first sample of the image
                        // cursor sample will be displayed between the origin
                        // and the center
                        shared_state.first_visible_sample = Some(self.image.lower);
                    }
                } else if self.image.lower + req_sample_window < self.image.upper {
                    // image range is larger than req_sample_window
                    // render the end of the available samples
                    shared_state.first_visible_sample = Some(self.image.upper - req_sample_window);
                } else {
                    // image range is smaller than req_sample_window
                    // render the available samples
                    shared_state.first_visible_sample = Some(self.image.lower);
                }

                return;
            }
        };

        match shared_state.mode {
            Mode::Scrollable(CursorDirection::Forward) => {
                if shared_state.cursor_sample < first_visible_sample + half_req_sample_window {
                    return;
                }

                if self.image.upper < first_visible_sample + req_sample_window {
                    return;
                }

                // Move the image so that the cursor is centered
                shared_state.first_visible_sample =
                    Some(shared_state.cursor_sample - half_req_sample_window);
            }
            Mode::Scrollable(CursorDirection::Backward(prev_sample)) => {
                if shared_state.cursor_sample <= first_visible_sample + half_req_sample_window {
                    // No longer in second half
                    debug!(
                        "{}_refresh_window cursor direction: Backward -> Forward",
                        self.image.id
                    );
                    shared_state.mode = Mode::Scrollable(CursorDirection::Forward);

                    return;
                }

                // still in second half
                if first_visible_sample + req_sample_window < self.image.upper {
                    // and there is still overhead
                    // => progressively get cursor back to center
                    let previous_offset = prev_sample - first_visible_sample;
                    let delta_cursor = shared_state.cursor_sample - prev_sample;
                    shared_state.first_visible_sample =
                        Some(shared_state.cursor_sample - previous_offset + delta_cursor);

                    shared_state.mode =
                        Mode::Scrollable(CursorDirection::Backward(shared_state.cursor_sample));
                } else {
                    // Not enough overhead to get cursor back to center
                    shared_state.mode = Mode::Scrollable(CursorDirection::Forward);
                }
            }
            Mode::Frozen => (),
        }
    }

    // Update rendering conditions
    pub fn update_conditions(&mut self, duration_per_1000px: Duration, width: i32, height: i32) {
        let WaveformRenderer {
            state,
            image,
            dimensions,
            ..
        } = self;
        let mut dim = dimensions.write().unwrap();

        let (mut scale_num, mut scale_denom) = if duration_per_1000px == dim.req_duration_per_1000px
        {
            (0, 0)
        } else {
            let prev_duration = dim.req_duration_per_1000px;
            dim.req_duration_per_1000px = duration_per_1000px;
            debug!(
                "{}_update_conditions duration/1000px {} -> {}",
                image.id, prev_duration, dim.req_duration_per_1000px,
            );
            Self::update_sample_step(state, &mut dim, image);
            (duration_per_1000px.as_usize(), prev_duration.as_usize())
        };

        if let Some(prev_width) = image.update_width(width) {
            if width != 0 {
                scale_num = prev_width as usize;
                scale_denom = width as usize;

                dim.width_f = f64::from(width);
            }
        }

        image.update_height(height);

        if scale_denom != 0 {
            Self::update_sample_window(&mut dim, &image);

            // update first sample in order to match new conditions
            if scale_num != 0 {
                let mut shared_state = self.shared_state.write().unwrap();

                if let Some(first_visible_sample) = shared_state.first_visible_sample {
                    let new_first_visible_sample =
                        first_visible_sample + dim.req_sample_window.scale(scale_num, scale_denom);

                    if new_first_visible_sample > image.sample_step {
                        let lower = image.lower;
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
                            image.id,
                            image.lower,
                            image.upper,
                            dim.req_sample_window,
                            first_visible_sample,
                            new_first_visible_sample,
                            image.sample_step,
                            shared_state.cursor_sample,
                        );

                        shared_state.first_visible_sample = Some(new_first_visible_sample);
                    } else {
                        // first_visible_sample can be snapped to the beginning
                        // => have refresh and update_first_visible_sample
                        // compute the best range for this situation
                        shared_state.first_visible_sample = None;
                    }
                }
            }
        }
    }

    #[inline]
    fn update_sample_step(
        state: &SampleExtractionState,
        dim: &mut Dimensions,
        image: &mut WaveformImage,
    ) {
        // compute a sample step which will produce an integral number of
        // samples per pixel or an integral number of pixels per samples

        dim.sample_step_f = if dim.req_duration_per_1000px >= state.duration_per_1000_samples {
            (dim.req_duration_per_1000px.as_f64() / state.duration_per_1000_samples.as_f64())
                .floor()
        } else {
            1f64 / (state.duration_per_1000_samples.as_f64() / dim.req_duration_per_1000px.as_f64())
                .ceil()
        };

        image.update_sample_step(dim.sample_step_f);
        dim.is_initialized = image.sample_step != SampleIndexRange::default();
    }

    #[inline]
    fn update_sample_window(dim: &mut Dimensions, image: &WaveformImage) {
        // force sample window to an even number of samples so that the cursor can be centered
        // and make sure to cover at least the requested width
        let half_req_sample_window = (dim.sample_step_f * dim.width_f / 2f64) as usize;
        let req_sample_window = half_req_sample_window * 2;

        if req_sample_window != dim.req_sample_window.as_usize() {
            debug!(
                "{}_update_sample_window smpl.window prev. {}, new {}",
                image.id, dim.req_sample_window, req_sample_window
            );
        }

        dim.req_sample_window = req_sample_window.into();
        dim.half_req_sample_window = half_req_sample_window.into();
        dim.quarter_req_sample_window = (half_req_sample_window / 2).into();
    }

    // Get the waveform as an image in current conditions.
    pub fn image(&mut self) -> Option<(&mut Image, ImagePositions)> {
        let (req_sample_window, width_f) = {
            let dim = self.dimensions.read().unwrap();
            (dim.req_sample_window, dim.width_f)
        };

        let shared_state = self.shared_state.read().unwrap();

        let first_visible_sample = match shared_state.first_visible_sample {
            Some(first_visible_sample) => {
                if first_visible_sample > self.image.upper
                    || first_visible_sample < self.image.lower
                {
                    return None;
                }

                first_visible_sample
            }
            None => {
                debug!("{}_image first_visible_sample not available", self.image.id);
                return None;
            }
        };

        let sample_duration = self.state.sample_duration;

        let first_index =
            (first_visible_sample - self.image.lower).step_range(self.image.sample_step);
        let first = SamplePosition {
            x: first_index as f64 * self.image.x_step_f,
            ts: first_visible_sample.as_ts(sample_duration),
        };

        let cursor = shared_state
            .cursor_sample
            .checked_sub(first_visible_sample)
            .and_then(|range_to_cursor| {
                if range_to_cursor <= req_sample_window {
                    let delta_index = range_to_cursor.step_range(self.image.sample_step);
                    Some(SamplePosition {
                        x: delta_index as f64 * self.image.x_step_f,
                        ts: shared_state.cursor_ts,
                    })
                } else {
                    None
                }
            });

        let last = {
            let visible_sample_range = self.image.upper - first_visible_sample;
            if visible_sample_range > req_sample_window {
                SamplePosition {
                    x: width_f,
                    ts: (first_visible_sample + req_sample_window)
                        .as_ts(self.state.sample_duration),
                }
            } else {
                let delta_index = visible_sample_range.step_range(self.image.sample_step);
                SamplePosition {
                    x: delta_index as f64 * self.image.x_step_f,
                    ts: self.image.upper.as_ts(self.state.sample_duration),
                }
            }
        };

        let sample_step = self.image.sample_step_f;

        Some((
            self.image.image(),
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
        let (cursor_sample, first_visible_sample) = {
            let shared_state = self.shared_state.read().unwrap();

            if shared_state.hold_rendering {
                return;
            }

            (
                shared_state.cursor_sample,
                shared_state.first_visible_sample,
            )
        };

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
                cursor_sample,
                self.image.lower,
                self.image.upper,
                audio_buffer.lower,
                audio_buffer.upper,
                segment_lower,
            );

            (segment_lower, audio_buffer.upper)
        };

        let (req_sample_window, half_req_sample_window, quarter_req_sample_window) = {
            let dim = self.dimensions.read().unwrap();
            (
                dim.req_sample_window,
                dim.half_req_sample_window,
                dim.quarter_req_sample_window,
            )
        };

        // Second step: find the range to display
        let extraction_range = if upper - lower <= req_sample_window {
            // image can use the full window
            trace!(
                "{}_render using full window, range [{}, {}]",
                self.image.id,
                lower,
                upper,
            );

            Some((lower, upper))
        } else if cursor_sample <= lower || cursor_sample >= upper {
            trace!(
                concat!(
                    "{}_render cursor not in the window: first_visible_sample ",
                    "{:?}, cursor {}, merged range [{}, {}]",
                ),
                self.image.id,
                first_visible_sample,
                cursor_sample,
                lower,
                upper,
            );

            // use defaults
            None
        } else {
            match first_visible_sample {
                Some(first_visible_sample) => {
                    if cursor_sample >= first_visible_sample
                        && cursor_sample < first_visible_sample + req_sample_window
                    {
                        // cursor is in the window => keep it
                        trace!(
                            concat!(
                                "{}_render cursor in the window: first_visible_sample ",
                                "{}, cursor {}, merged range [{}, {}]",
                            ),
                            self.image.id,
                            first_visible_sample,
                            cursor_sample,
                            lower,
                            upper,
                        );

                        Some((
                            first_visible_sample,
                            upper.min(
                                first_visible_sample + req_sample_window + half_req_sample_window,
                            ),
                        ))
                    } else {
                        debug!(
                            concat!(
                                "{}_render first_visible_sample ",
                                "{} and cursor {} not in the same range [{}, {}]",
                            ),
                            self.image.id, first_visible_sample, cursor_sample, lower, upper,
                        );

                        // use defaults
                        None
                    }
                }
                None => {
                    if cursor_sample > lower + half_req_sample_window && cursor_sample < upper {
                        // cursor can be centered or is in second half of the window
                        if cursor_sample + half_req_sample_window < upper {
                            // cursor can be centered
                            trace!(
                                "{}_render centering cursor: {}",
                                self.image.id,
                                cursor_sample
                            );

                            Some((
                                cursor_sample - half_req_sample_window,
                                upper.min(cursor_sample + req_sample_window),
                            ))
                        } else {
                            // cursor in second half
                            trace!(
                                "{}_render cursor: {} in second half",
                                self.image.id,
                                cursor_sample
                            );

                            // attempt to get an optimal range
                            if upper > lower + req_sample_window + quarter_req_sample_window {
                                Some((upper - req_sample_window - quarter_req_sample_window, upper))
                            } else if upper > lower + req_sample_window {
                                Some((upper - req_sample_window, upper))
                            } else {
                                // use defaults
                                None
                            }
                        }
                    } else {
                        trace!(
                            "{}_render cursor {} in first half or before range [{}, {}]",
                            self.image.id,
                            cursor_sample,
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
                    .min(audio_buffer.lower + req_sample_window + half_req_sample_window),
            )
        });

        self.image.render(audio_buffer, lower, upper);
    }
}

impl SampleExtractor for WaveformRenderer {
    fn extraction_state(&self) -> &SampleExtractionState {
        &self.state
    }

    fn extraction_state_mut(&mut self) -> &mut SampleExtractionState {
        &mut self.state
    }

    fn lower(&self) -> SampleIndex {
        self.shared_state
            .read()
            .unwrap()
            .first_visible_sample
            .map_or(self.image.lower, |sample| {
                let dim = self.dimensions.read().unwrap();

                if sample > dim.half_req_sample_window {
                    sample - dim.half_req_sample_window
                } else {
                    sample
                }
            })
    }

    fn req_sample_window(&self) -> Option<SampleIndexRange> {
        let dim = self.dimensions.read().unwrap();

        if dim.req_sample_window == SampleIndexRange::default() {
            None
        } else {
            Some(dim.req_sample_window)
        }
    }

    fn cleanup(&mut self) {
        // clear for reuse
        debug!("{}_cleanup", self.image.id);

        self.state.cleanup();
        self.reset();
    }

    fn set_sample_duration(&mut self, per_sample: Duration, per_1000_samples: Duration) {
        let WaveformRenderer {
            state,
            image,
            dimensions,
            ..
        } = self;

        debug!("{}_set_sample_duration per_sample {}", image.id, per_sample,);

        state.sample_duration = per_sample;
        state.duration_per_1000_samples = per_1000_samples;

        let mut dim = dimensions.write().unwrap();

        Self::reset_sample_conditions(&mut dim, image);
        Self::update_sample_step(state, &mut dim, image);
        Self::update_sample_window(&mut dim, image);
    }

    fn set_channels(&mut self, channels: &[AudioChannel]) {
        self.image.set_channels(channels);
    }

    fn update_concrete_state(&mut self, other: &mut WaveformRenderer) {
        // FIXME consider using a RwLock
        self.image.update_from_other(&mut other.image);
    }

    // This is the entry point for the waveform update.
    // This function tries to merge the samples added to the AudioBuffer
    // since last extraction and adapts to the evolving conditions of
    // the playback position and target rendering dimensions and
    // resolution.
    fn extract_samples(&mut self, audio_buffer: &AudioBuffer) {
        if self.dimensions.read().unwrap().req_sample_window == SampleIndexRange::default() {
            // conditions not defined yet
            return;
        }

        self.render(audio_buffer);

        let mut shared_state = self.shared_state.write().unwrap();

        if audio_buffer.contains_eos() && !self.image.contains_eos {
            // there won't be any refresh on behalf of audio_buffer
            // and image will still need more sample if playback continues
            debug!(
                "{}_extract_samples setting playback_needs_refresh",
                self.image.id
            );

            shared_state.playback_needs_refresh = true;
        } else {
            if shared_state.playback_needs_refresh {
                debug!(
                    "{}_extract_samples resetting playback_needs_refresh",
                    self.image.id
                );
            }
            shared_state.playback_needs_refresh = false;
        }
    }

    fn refresh(&mut self, audio_buffer: &AudioBuffer) {
        if self.image.is_initialized {
            // Note: current state is up to date (updated from DoubleAudioBuffer)

            self.render(audio_buffer);

            let mut shared_state = self.shared_state.write().unwrap();

            if shared_state.playback_needs_refresh && self.image.contains_eos {
                debug!("{}_refresh resetting playback_needs_refresh", self.image.id);
            }

            shared_state.playback_needs_refresh = !self.image.contains_eos;
        } else {
            debug!("{}_refresh image not ready", self.image.id);
        }
    }
}
