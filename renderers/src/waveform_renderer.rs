use log::{debug, trace};

use std::sync::{Arc, Mutex, RwLock};

use media::{
    sample_extractor, AudioBuffer, AudioChannel, DoubleAudioBuffer, SampleExtractor, SampleIndex,
    SampleIndexRange, Timestamp,
};

use metadata::Duration;

use super::{waveform::Dimensions, waveform_image, Image, WaveformImage};

pub struct DoubleWaveformRenderer;

impl DoubleWaveformRenderer {
    pub fn new_dbl_audio_buffer(buffer_duration: Duration) -> DoubleAudioBuffer<WaveformRenderer> {
        let shared_state = Arc::new(RwLock::new(SharedState::default()));
        let dimensions = Arc::new(RwLock::new(Dimensions::default()));
        let extract_state = Arc::new(RwLock::new(sample_extractor::State::default()));
        let channel_colors = Arc::new(Mutex::new(waveform_image::ChannelColors::default()));
        let secondary_image = Arc::new(Mutex::new(None));

        DoubleAudioBuffer::new(
            buffer_duration,
            Box::new(WaveformRenderer::new(
                1,
                Arc::clone(&shared_state),
                Arc::clone(&dimensions),
                Arc::clone(&extract_state),
                Arc::clone(&channel_colors),
                Arc::clone(&secondary_image),
            )),
            Box::new(WaveformRenderer::new(
                2,
                shared_state,
                dimensions,
                extract_state,
                channel_colors,
                secondary_image,
            )),
        )
    }
}

#[derive(Debug, Default)]
pub struct SamplePosition {
    pub x: f64,
    pub ts: Timestamp,
}

#[derive(Debug, Default)]
pub struct ImagePositions {
    pub offset: SamplePosition,
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
    pub image: WaveformImage,
    dimensions: Arc<RwLock<Dimensions>>,

    shared_state: Arc<RwLock<SharedState>>,
    extract_state: Arc<RwLock<sample_extractor::State>>,
}

impl WaveformRenderer {
    fn new(
        id: usize,
        shared_state: Arc<RwLock<SharedState>>,
        dimensions: Arc<RwLock<Dimensions>>,
        extract_state: Arc<RwLock<sample_extractor::State>>,
        channel_colors: Arc<Mutex<waveform_image::ChannelColors>>,
        secondary_image: Arc<Mutex<Option<Image>>>,
    ) -> Self {
        WaveformRenderer {
            image: WaveformImage::new(id, channel_colors, secondary_image),
            dimensions,

            shared_state,
            extract_state,
        }
    }

    pub fn reset(&mut self) {
        debug!("{}_reset", self.image.id);

        self.shared_state.write().unwrap().reset();
        self.dimensions.write().unwrap().reset();
        self.image.cleanup();
    }

    fn reset_sample_conditions(&mut self) {
        debug!("{}_reset_sample_conditions", self.image.id);

        self.dimensions.write().unwrap().reset();
        self.image.cleanup_sample_conditions();
    }

    pub fn limits_as_ts(&self) -> (Timestamp, Timestamp) {
        let sample_duration = self.dimensions.read().unwrap().sample_duration;

        (
            self.image.lower.as_ts(sample_duration),
            self.image.upper.as_ts(sample_duration),
        )
    }

    pub fn half_window_duration(&self) -> Duration {
        let d = self.dimensions.read().unwrap();
        d.half_req_sample_window.duration(d.sample_duration)
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

    pub fn seek_done(&mut self, ts: Timestamp) {
        let cursor_sample = ts.sample_index(self.dimensions.read().unwrap().sample_duration);

        let mut shared_state = self.shared_state.write().unwrap();

        shared_state.cursor_sample = cursor_sample;
        shared_state.cursor_ts = ts;

        let first_visible_sample = match shared_state.first_visible_sample {
            Some(first_visible_sample) => first_visible_sample,
            None => return,
        };

        let (req_sample_window, half_req_sample_window) = {
            let dim = self.dimensions.read().unwrap();
            (dim.req_sample_window, dim.half_req_sample_window)
        };

        if shared_state.cursor_sample >= first_visible_sample + req_sample_window
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
            }
        }
    }

    #[inline]
    fn cursor(&self, sample_duration: Duration) -> Option<(Timestamp, SampleIndex)> {
        // Keep this in as a statement as `current_ts()` locks the extraction state.
        // We want to avoid race conditions when locking `SharedState`.
        let current_ts = self.current_ts();

        current_ts.map(|ts| {
            let mut sample = ts.sample_index(sample_duration);
            if self.image.contains_eos && sample >= self.image.upper {
                sample = self.image.upper;
                sample
                    .try_dec()
                    .expect("adjusting cursor_sample to last sample in stream");
            }

            (ts, sample)
        })
    }

    /// Refreshes the cursor position.
    pub fn refresh_cursor(&mut self) {
        let sample_duration = self.dimensions.read().unwrap().sample_duration;

        if let Some((ts, sample)) = self.cursor(sample_duration) {
            let mut shared_state = self.shared_state.write().unwrap();
            shared_state.cursor_sample = sample;
            shared_state.cursor_ts = ts;
        }
    }

    /// Refreshes the waveform position.
    ///
    /// Refreshes the cursor and computes the first sample to display depending on current mode.
    pub fn refresh(&mut self) {
        if !self.image.is_ready {
            debug!(
                "{}_update_first_visible_sample image not ready",
                self.image.id
            );

            self.shared_state.write().unwrap().first_visible_sample = None;
            return;
        }

        let (sample_duration, req_sample_window, half_req_sample_window) = {
            let dim = self.dimensions.read().unwrap();
            (
                dim.sample_duration,
                dim.req_sample_window,
                dim.half_req_sample_window,
            )
        };

        // Keep this order and separation so as to avoid race conditions locking the states.
        let cursor = self.cursor(sample_duration);
        let mut shared_state = self.shared_state.write().unwrap();
        if let Some((ts, sample)) = cursor {
            shared_state.cursor_sample = sample;
            shared_state.cursor_ts = ts;
        }

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

    /// Updates rendering conditions
    pub fn update_conditions(
        &mut self,
        req_duration_per_1000px: Duration,
        width: i32,
        height: i32,
    ) {
        let mut d = self.dimensions.write().unwrap();

        let mut must_update_sample_window = false;

        if req_duration_per_1000px != d.req_duration_per_1000px {
            let prev_req_duration = d.req_duration_per_1000px;
            d.req_duration_per_1000px = req_duration_per_1000px;
            debug!(
                "{}_update_conditions duration/1000px {} -> {}",
                self.image.id, prev_req_duration, d.req_duration_per_1000px,
            );
            self.update_sample_step(&mut d);

            if width > 0 {
                must_update_sample_window = true;
            }
        }

        if width != d.req_width {
            d.force_redraw_1 = true;
            d.force_redraw_2 = true;

            debug!(
                "{}_update_conditions prev. width {} -> {}",
                self.image.id, d.req_width, width,
            );

            if req_duration_per_1000px > Duration::default() {
                must_update_sample_window = true;
            }

            d.req_width = width;
            d.req_width_f = f64::from(width);
        }

        if height != d.req_height {
            d.force_redraw_1 = true;
            d.force_redraw_2 = true;

            debug!(
                "{}_update_conditions prev. height {} -> {}",
                self.image.id, d.req_height, height,
            );
            d.req_height = height;
        }

        if must_update_sample_window {
            let req_sample_window_prev = d.req_sample_window;
            self.update_sample_window(&mut d);

            if req_sample_window_prev == SampleIndexRange::default() {
                return;
            }

            // update first sample in order to match new conditions
            let (sample_step, req_sample_window, half_req_sample_window) =
                (d.sample_step, d.req_sample_window, d.half_req_sample_window);

            // Prevent race conditions
            drop(d);
            let mut shared_state = self.shared_state.write().unwrap();

            if let Some(first_visible_sample) = shared_state.first_visible_sample {
                // rebase the waveform so that the cursor appears at the same position on screen

                if shared_state.cursor_sample < first_visible_sample {
                    // compute the best range for this situation
                    shared_state.first_visible_sample = None;
                    return;
                }

                let cursor_offset_prev = shared_state.cursor_sample - first_visible_sample;
                if cursor_offset_prev < half_req_sample_window {
                    return;
                }

                let new_first_visible_sample = shared_state
                    .cursor_sample
                    .saturating_sub_range(
                        cursor_offset_prev.scale(req_sample_window, req_sample_window_prev),
                    )
                    .snap_to(sample_step);

                let new_first_visible_sample = if new_first_visible_sample > self.image.lower {
                    new_first_visible_sample
                } else {
                    self.image.lower
                };

                debug!(
                    concat!(
                        "{}_rebase range [{}, {}], window {}, ",
                        "first {} -> {}, sample_step {}, cursor_sample {}",
                    ),
                    self.image.id,
                    self.image.lower,
                    self.image.upper,
                    req_sample_window,
                    first_visible_sample,
                    new_first_visible_sample,
                    sample_step,
                    shared_state.cursor_sample,
                );

                shared_state.first_visible_sample = Some(new_first_visible_sample);
            }
        }
    }

    #[inline]
    fn update_sample_step(&self, d: &mut Dimensions) {
        // compute a sample step which will produce an integral number of
        // samples per pixel or an integral number of pixels per samples
        let prev_sample_step_f = d.sample_step_f;

        d.sample_step_f = if d.req_duration_per_1000px >= d.duration_per_1000_samples {
            (d.req_duration_per_1000px.as_f64() / d.duration_per_1000_samples.as_f64()).floor()
        } else {
            1f64 / (d.duration_per_1000_samples.as_f64() / d.req_duration_per_1000px.as_f64())
                .ceil()
        };

        d.sample_step = (d.sample_step_f as usize).max(1).into();
        d.x_step_f = if d.sample_step_f < 1f64 {
            (1f64 / d.sample_step_f).round()
        } else {
            1f64
        };
        d.x_step = d.x_step_f as usize;

        let force_redraw = (d.sample_step_f - prev_sample_step_f).abs() > 0.01f64;
        d.force_redraw_1 |= force_redraw;
        d.force_redraw_2 |= force_redraw;
    }

    #[inline]
    fn update_sample_window(&self, d: &mut Dimensions) {
        let half_req_sample_window = (d.sample_step_f * d.req_width_f / 2f64) as usize;
        let req_sample_window = half_req_sample_window * 2;

        if req_sample_window != d.req_sample_window.as_usize() {
            debug!(
                "{}_update_sample_window smpl.window prev. {} -> {}",
                self.image.id, d.req_sample_window, req_sample_window
            );
        }

        d.req_sample_window = req_sample_window.into();
        d.half_req_sample_window = half_req_sample_window.into();
        d.quarter_req_sample_window = (half_req_sample_window / 2).into();

        debug!("{}_update_sample_window {:?}", self.image.id, *d);
    }

    // Get the waveform as an image in current conditions.
    pub fn image(&mut self) -> Option<(&Image, ImagePositions)> {
        let d = *self.dimensions.read().unwrap();

        let shared_state = self.shared_state.read().unwrap();

        let first_sample = shared_state
            .first_visible_sample
            .filter(|first_visible_sample| {
                *first_visible_sample < self.image.upper
                    && *first_visible_sample >= self.image.lower
            })
            .unwrap_or(self.image.lower);

        let first_offset = (first_sample - self.image.lower).step_range(d.sample_step);
        let offset = SamplePosition {
            x: first_offset as f64 * d.x_step_f,
            ts: first_sample.as_ts(d.sample_duration),
        };

        // Only display cursor if first_visible_sample is known
        let cursor = shared_state
            .first_visible_sample
            .and_then(|first_visible_sample| {
                shared_state
                    .cursor_sample
                    .checked_sub(first_visible_sample)
                    .filter(|range_to_cursor| *range_to_cursor <= d.req_sample_window)
                    .map(|range_to_cursor| {
                        let delta_index = range_to_cursor.step_range(d.sample_step);
                        SamplePosition {
                            x: delta_index as f64 * d.x_step_f,
                            ts: shared_state.cursor_ts,
                        }
                    })
            });

        let last = {
            let visible_sample_range = self.image.upper - first_sample;
            if visible_sample_range > d.req_sample_window {
                SamplePosition {
                    x: d.req_width_f,
                    ts: (first_sample + d.req_sample_window).as_ts(d.sample_duration),
                }
            } else {
                let delta_index = visible_sample_range.step_range(d.sample_step);
                SamplePosition {
                    x: delta_index as f64 * d.x_step_f,
                    ts: self.image.upper.as_ts(d.sample_duration),
                }
            }
        };

        Some((
            self.image.image(),
            ImagePositions {
                offset,
                cursor,
                last,
                sample_duration: d.sample_duration,
                sample_step: d.sample_step_f,
            },
        ))
    }

    fn render(&mut self, audio_buffer: &AudioBuffer) {
        let (cursor_sample, first_visible_sample) = {
            let shared_state = self.shared_state.read().unwrap();

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

        // Get a consistent snapshot of the dimensions while we render
        let d = {
            let mut d = self.dimensions.write().unwrap();
            let d_copy = *d;

            // If a redraw is requested in this conditions, we will take
            // care of it in `self.image.render()`.
            if self.image.id == 1 {
                d.force_redraw_1 = false
            } else {
                d.force_redraw_2 = false
            };

            d_copy
        };

        self.image.render(d, audio_buffer, lower, upper);
    }
}

impl SampleExtractor for WaveformRenderer {
    fn with_state<Out>(&self, f: impl FnOnce(&sample_extractor::State) -> Out) -> Out {
        let extract_state = self.extract_state.read().unwrap();
        f(&extract_state)
    }

    fn with_state_mut<Out>(&mut self, f: impl FnOnce(&mut sample_extractor::State) -> Out) -> Out {
        let mut extract_state = self.extract_state.write().unwrap();
        f(&mut extract_state)
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
        let d = self.dimensions.read().unwrap();

        if d.req_sample_window == SampleIndexRange::default() {
            None
        } else {
            Some(d.req_sample_window)
        }
    }

    fn cleanup(&mut self) {
        // clear for reuse
        debug!("{}_cleanup", self.image.id);

        self.extract_state.write().unwrap().cleanup();
        self.reset();
    }

    fn set_sample_duration(&mut self, per_sample: Duration, per_1000_samples: Duration) {
        self.reset_sample_conditions();

        let mut d = self.dimensions.write().unwrap();

        d.sample_duration = per_sample;
        d.duration_per_1000_samples = per_1000_samples;

        self.update_sample_step(&mut d);
        self.update_sample_window(&mut d);
    }

    fn set_channels(&mut self, channels: impl Iterator<Item = AudioChannel>) {
        self.image.set_channels(channels);
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

            // FIXME there should be one for each waveform
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
}
