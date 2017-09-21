extern crate cairo;

#[cfg(feature = "profiling-waveform-buffer")]
use chrono::Utc;

use std::any::Any;

use std::sync::{Arc, Mutex};

use ::media::{AudioBuffer, SAMPLES_NORM};

use ::media::{DoubleSampleExtractor, SamplesExtractor};
use ::media::samples_extractor::SamplesExtractionState;

pub const BACKGROUND_COLOR: (f64, f64, f64) = (0.2f64, 0.2235f64, 0.2314f64);

pub struct DoubleWaveformBuffer {}
impl DoubleWaveformBuffer {
    pub fn new(
        exposed_mtx: &Arc<Mutex<Box<SamplesExtractor>>>
    ) -> DoubleSampleExtractor {
        DoubleSampleExtractor::new(
            Arc::clone(exposed_mtx),
            Box::new(WaveformBuffer::new())
        )
    }
}

pub struct WaveformBuffer {
    state: SamplesExtractionState,
    buffer_sample_window: usize,

    width: i32,
    height: i32,
    pub exposed_image: Option<cairo::ImageSurface>,
    working_image: Option<cairo::ImageSurface>,
}

impl WaveformBuffer {
    pub fn new() -> Self {
        WaveformBuffer {
            state: SamplesExtractionState::new(),
            buffer_sample_window: 0,

            width: 0,
            height: 0,
            exposed_image: None,
            working_image: None,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        self.cleanup_state();
        self.buffer_sample_window = 0;
        self.width = 0;
        self.height = 0;
        self.exposed_image = None;
        self.working_image = None;
    }

    pub fn get_position_from_x(&mut self, x: f64) -> Option<u64> {
        match self.get_first_visible_sample() {
            Some(first_visible_sample) =>
                Some(
                    (
                        first_visible_sample as u64
                        + (x as u64) * (self.state.sample_step as u64)
                    ) * self.state.sample_duration_u
                ),
            None => None,
        }
    }

    fn get_first_visible_sample(&mut self) -> Option<usize> {
        if self.exposed_image.is_some() {
            let state = &mut self.state;
            state.query_current_sample();

            if state.current_sample >= state.first_sample {
                // current sample appears after first buffer sample
                if state.current_sample + state.half_requested_sample_window <= state.last_sample {
                    // current sample fits in the first half of the window with last sample further
                    if state.current_sample > state.first_sample + state.half_requested_sample_window {
                        // current sample can be centered
                        Some(state.current_sample - state.half_requested_sample_window)
                    } else {
                        // set origin to the first sample in the buffer
                        // current sample will be displayed between the origin
                        // and the center
                        Some(state.first_sample)
                    }
                } else if state.current_sample <= state.last_sample + 2 * state.sample_step {
                    // current sample can fit in the second half of the window
                    // (take a margin due to rounding to sample_step)
                    if self.buffer_sample_window >= state.requested_sample_window {
                        // buffer window is larger than requested_sample_window
                        // set last buffer to the right
                        Some(state.last_sample - state.requested_sample_window)
                    } else {
                        // buffer window is smaller than requested_sample_window
                        // set first sample to the left
                        Some(state.first_sample)
                    }
                } else {
                    // current sample appears further than last sample
                    None
                }
            }
            else {
                // current sample appears before buffer first sample
                None
            }
        } else {
            // no image available yet
            None
        }
    }

    pub fn update_conditions(&mut self,
        duration: u64,
        width: i32,
        height: i32,
    ) -> Option<(usize, usize)> // (x_offset, current_x)
    {
        {
            let state = &mut self.state;

            self.width = width;
            self.height = height;

            let width = width as u64;
            // resolution
            state.requested_step_duration =
                if duration > width {
                    duration / width
                } else {
                    1
                };

            state.requested_sample_window = (
                duration as f64 / state.sample_duration
            ).round() as usize;
            state.half_requested_sample_window = state.requested_sample_window / 2;
        }

        match self.get_first_visible_sample() {
            Some(first_visible_sample) => {
                let state = &self.state;
                Some((
                    (first_visible_sample - state.first_sample) / state.sample_step, // x_offset
                    (state.current_sample - first_visible_sample) / state.sample_step, // current_x
                ))
            },
            None => None,
        }
    }

    // This function is called on a working buffer
    // which means that self.exposed_image image is the image
    // that was previously exposed to the UI
    // this also means that we can safely deal with both
    // images since none of them is exposed at this very moment
    fn update_extraction(&mut self,
        audio_buffer: &AudioBuffer,
        first_sample: usize,
        last_sample: usize,
        sample_step: usize,
    ) {
        #[cfg(feature = "profiling-waveform-buffer")]
        let start = Utc::now();

        let state = &mut self.state;

        let extraction_samples_window = (last_sample - first_sample) / sample_step;

        let mut must_redraw = true || state.sample_step != sample_step;
        if !must_redraw && first_sample >= state.first_sample
        && last_sample <= state.last_sample
        {   // traget extraction fits in previous extraction
            return;
        } else if first_sample + extraction_samples_window < state.first_sample
            || first_sample > state.last_sample
        {   // current samples extraction doesn't overlap with samples in previous image
            must_redraw = true;
        }

        let working_image = {
            let mut can_reuse = false;
            let target_width = (extraction_samples_window as i32).max(self.width);

            if let Some(ref working_image) = self.working_image {
                if self.height != working_image.get_height() {
                    // height has changed => scale samples amplitude accordingly
                    must_redraw = true;
                }

                if target_width <= working_image.get_width()
                && self.height <= working_image.get_height() {
                    // expected dimensions fit in current working image => reuse it
                    can_reuse = true;
                }
            }

            if can_reuse {
                self.working_image.take().unwrap()
            } else {
                cairo::ImageSurface::create(
                    cairo::Format::Rgb24,
                    target_width,
                    self.height
                ).expect("WaveformBuffer: couldn't create image surface in update_extraction")
            }
        };

        let cr = cairo::Context::new(&working_image);
        let (mut sample_iter, mut x, clear_limit) =
            if must_redraw {
                // Initialization or resolution has changed or seek requested
                // redraw the whole range

                // clear the image
                cr.set_source_rgb(
                    BACKGROUND_COLOR.0,
                    BACKGROUND_COLOR.1,
                    BACKGROUND_COLOR.2
                );
                cr.paint();

                state.sample_step = sample_step;
                state.first_sample = first_sample;
                state.last_sample = last_sample;

                (
                    audio_buffer.iter(first_sample, last_sample, sample_step),
                    0f64,
                    0f64,
                )
            } else {
                // can reuse previous context
                let previous_image = self.exposed_image.take()
                    .expect("WaveformBuffer: no exposed_image while updating");

                let (image_offset, sample_iter, x, clear_limit) = {
                    // Note: condition first_sample >= self.state.first_sample
                    //                 && last_sample <= self.state.last_sample
                    // (traget extraction fits in previous extraction)
                    // already checked

                    if first_sample < state.first_sample {
                        // append samples before previous first sample
                        println!("appending samples before previous first sample");

                        let image_width_as_samples =
                            working_image.get_width() as usize * sample_step;

                        let previous_first_sample = state.first_sample;
                        state.first_sample = first_sample;
                        state.last_sample = state.last_sample.min(
                            first_sample + image_width_as_samples
                        );

                        // shift previous image to the right
                        let image_offset = (
                            (previous_first_sample - first_sample) / sample_step
                        ) as f64;

                        (
                            image_offset,
                            audio_buffer.iter(first_sample, previous_first_sample, sample_step), // sample_iter
                            0f64, // first x to draw
                            image_offset, // clear_limit
                        )
                    } else {
                        // first_sample >= state.first_sample
                        // Note: due to previous conditions tested before,
                        // this also implies:
                        assert!(last_sample > state.last_sample);

                        let previous_first_sample = state.first_sample;
                        let previous_last_sample = state.last_sample;
                        // Note: image width is such a way that samples in
                        // (first_sample, last_sample) can all be rendered
                        state.first_sample = first_sample;
                        state.last_sample = last_sample;

                        // shift previous image to the left (if necessary)
                        let image_offset = -((
                            (first_sample - previous_first_sample) / sample_step
                        ) as f64);

                        // append samples after previous last sample
                        let first_sample_to_draw = previous_last_sample.max(first_sample);

                        // prepare to add remaining samples
                        (
                            image_offset,
                            audio_buffer.iter(first_sample_to_draw, last_sample, sample_step), // sample_iter
                            (
                                (first_sample_to_draw - previous_first_sample) / sample_step
                            ) as f64 + image_offset, // first x to draw
                            f64::from(working_image.get_width()), // clear_limit
                        )
                    }
                };

                cr.set_source_surface(&previous_image, image_offset, 0f64);
                cr.paint();

                // set image back, will be swapped later
                self.exposed_image = Some(previous_image);

                (sample_iter, x, clear_limit)
            };

        cr.scale(1f64, f64::from(self.height) / SAMPLES_NORM);

        if !must_redraw {
            // fill the rest of the image with background color
            cr.set_source_rgb(
                BACKGROUND_COLOR.0,
                BACKGROUND_COLOR.1,
                BACKGROUND_COLOR.2
            );
            cr.rectangle(x, 0f64, clear_limit - x, SAMPLES_NORM);
            cr.fill();
        } // else brackgroung already set while clearing the image

        if sample_iter.size_hint().0 > 0 {
            // Stroke selected samples
            cr.set_line_width(0.5f64);
            cr.set_source_rgb(0.8f64, 0.8f64, 0.8f64);

            let mut sample_value = *sample_iter.next().unwrap();
            for sample in sample_iter {
                cr.move_to(x, sample_value);
                x += 1f64;
                sample_value = *sample;
                cr.line_to(x, sample_value);
                cr.stroke();
            }
        }

        if let Some(previous_image) = self.exposed_image.take() {
            self.working_image = Some(previous_image);
        }
        self.exposed_image = Some(working_image);

        self.buffer_sample_window = state.last_sample - state.first_sample;

        #[cfg(feature = "profiling-waveform-buffer")]
        let end = Utc::now();

        #[cfg(feature = "profiling-waveform-buffer")]
        println!("waveform-buffer,{},{}",
            start.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
        );
    }
}

impl SamplesExtractor for WaveformBuffer {
    fn as_mut_any(&mut self) -> &mut Any {
        self
    }

    fn get_extraction_state(&self) -> &SamplesExtractionState {
        &self.state
    }

    fn get_extraction_state_mut(&mut self) -> &mut SamplesExtractionState {
        &mut self.state
    }

    fn can_extract(&self) -> bool {
        self.state.requested_sample_window > 0
    }

    fn extract_samples(&mut self, audio_buffer: &AudioBuffer) {
        let (first_visible_sample, last_sample, sample_step) = {
            let can_extract = self.can_extract();

            let state = self.get_extraction_state_mut();
            state.sample_duration = audio_buffer.sample_duration;
            state.sample_duration_u = audio_buffer.sample_duration_u;

            if !can_extract {
                return;
            }

            // use an integer number of samples per step
            let sample_step = (
                state.requested_step_duration / state.sample_duration_u
            ) as usize;

            if audio_buffer.samples.len() < sample_step {
                // buffer too small to render
                return;
            }

            // TODO: take advantage of the context:
            // samples already rendered, cursor position, etc.

            if state.seek_flag {
                // seeking => force current sample query
                //let previous_sample = state.current_sample;
                state.query_current_sample();
                // TODO: see how to smoothen the cursor's movements
                // when a seek is performed within the window
                // might be necessary to exchange data between buffers
                // could also use the x position from the WaveformBuffer
                // Note: this must be handle in the waveform buffer
                // as it has to do with UI
            }

            if audio_buffer.eos {
                // reached the end of stream
                // draw the end of the buffer to fit in the requested width
                // and adjust current position

                if state.current_sample >= audio_buffer.first_sample
                && state.current_sample < audio_buffer.last_sample
                && state.current_sample
                    >= audio_buffer.first_sample + state.half_requested_sample_window
                {
                    (
                        state.current_sample - state.half_requested_sample_window,
                        audio_buffer.last_sample,
                        sample_step
                    )
                } else {
                    (
                        audio_buffer.first_sample,
                        audio_buffer.last_sample,
                        sample_step
                    )
                }
            } else {
                if state.current_sample
                    >= audio_buffer.first_sample + state.half_requested_sample_window
                && state.current_sample + state.half_requested_sample_window
                    < audio_buffer.last_sample
                {
                    // regular case where the position can be centered on screen
                    // attempt to get a larger buffer in order to compensate
                    // for the delay when it will actually be drawn
                    let first_visible_sample =
                        state.current_sample - state.half_requested_sample_window;
                    (
                        first_visible_sample,
                        audio_buffer.last_sample.min(
                            first_visible_sample
                            + state.requested_sample_window + state.half_requested_sample_window
                        ),
                        sample_step
                    )
                } else {
                    // not enough samples for the requested window
                    // around current position
                    (
                        audio_buffer.first_sample,
                        audio_buffer.last_sample.min(
                            audio_buffer.first_sample
                            + state.requested_sample_window + state.half_requested_sample_window
                        ),
                        sample_step
                    )
                }
            }
        };

        // align requested first sample in order to keep a steady
        // offset between redraws. This allows using the same samples
        // for a given requested_step_duration and avoiding flickering
        // between redraws
        let mut first_sample =
            first_visible_sample / sample_step * sample_step;
        if first_sample < audio_buffer.first_sample {
            // first sample might be smaller than audio_buffer.first_sample
            // due to alignement on sample_step

            first_sample += sample_step;
        }

        self.update_extraction(
            audio_buffer,
            first_sample,
            last_sample / sample_step * sample_step,
            sample_step
        );
    }
}
