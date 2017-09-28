extern crate cairo;

use std::any::Any;

use std::sync::{Arc, Mutex};

use media::{AudioBuffer, DoubleAudioBuffer, SampleExtractor};

use media::sample_extractor::SampleExtractionState;

use super::WaveformImage;

pub struct DoubleWaveformBuffer {}

impl DoubleWaveformBuffer {
    pub fn new(buffer_duration: u64) -> Arc<Mutex<DoubleAudioBuffer>> {
        Arc::new(Mutex::new(
            DoubleAudioBuffer::new(
                buffer_duration,
                Box::new(WaveformBuffer::new()),
                Box::new(WaveformBuffer::new())
            )
        ))
    }
}

// A WaveformBuffer hosts one of the two buffers of the double buffering
// mechanism based on the SampleExtractor trait.
// It is responsible for preparing an up to date Waveform image which will be
// diplayed upon UI request. Up to date signifies that the Waveform image
// contains all the samples that can fit in the target window at the specified
// resolution for current playback position.
// Whenever possible, the WaveformBuffer attempts to have the Waveform scroll
// between frames with current playback position in the middle so that the
// user can seek forward or backward around current position.
// Since the nominal update is such that the Waveform scrolls between updates,
// two images are used. Whenever possible, the next image to expose to the UI
// is initialized as a translation of previous image and updated with missing
// samples.
pub struct WaveformBuffer {
    image: WaveformImage,
    state: SampleExtractionState,

    is_seeking: bool,
    current_sample: usize,
    first_visible_sample_lock: Option<i64>,
    sample_sought: Option<usize>,

    was_exposed: bool,
    req_sample_window: usize,
    half_req_sample_window: usize,
}

impl WaveformBuffer {
    pub fn new() -> Self {
        WaveformBuffer {
            state: SampleExtractionState::new(),
            image: WaveformImage::new(),

            is_seeking: false,
            current_sample: 0,
            first_visible_sample_lock: None,
            sample_sought: None,

            was_exposed: false,
            req_sample_window: 0,
            half_req_sample_window: 0,
        }
    }

    pub fn clear_exposed_status(&mut self) {
        self.was_exposed = false;
    }

    pub fn seek(&mut self, position: u64) {
        let sample_sought = (position as f64 / self.state.sample_duration) as usize;
        self.sample_sought = Some(sample_sought);
        if let Some(first_visible_sample_i) = self.first_visible_sample_lock {
            self.first_visible_sample_lock = {
                let first_visible_sample = first_visible_sample_i as usize;
                if first_visible_sample <= sample_sought
                && sample_sought < first_visible_sample + self.req_sample_window
                {   // first_visible_sample_lock is confirmed
                    Some(first_visible_sample_i)
                } else {
                    // first_visible_sample_lock no longer applicable
                    None
                }
            };
        }
        self.is_seeking = true;
        self.was_exposed = true;
    }

    // mark seek in window and return position if applicable
    // seeking must be confirmed by calling seek
    pub fn seek_in_window(&mut self, x: f64) -> Option<u64> {
        match self.get_first_visible_sample() {
            Some(first_visible_sample) => {
                self.first_visible_sample_lock = Some(first_visible_sample as i64);
                let sample_sought = first_visible_sample + (x as usize) * self.image.sample_step;
                Some((sample_sought as f64 * self.state.sample_duration) as u64)
            },
            None => None
        }
    }

    // Update to current position and compute the first
    // sample to present for display.
    fn get_first_visible_sample(&mut self) -> Option<usize> {
        if self.image.is_ready() {
            self.was_exposed = true;
            let previous_cursor = self.current_sample;
            self.current_sample = self.query_current_sample();

            if self.current_sample >= self.image.first_sample {
                // current sample appears after first buffer sample
                if let Some(first_visible_sample) = self.first_visible_sample_lock {
                    // adapt according to the evolution of the position
                    let center_offset = self.current_sample as i64
                        - self.half_req_sample_window as i64
                        - first_visible_sample;
                    if center_offset < -(self.image.sample_step as i64) {
                        // cursor in first half of the window
                        // keep origin on the first sample upon seek
                        // this is in case we move to the 2d half
                        self.sample_sought = Some(self.current_sample);
                        Some((first_visible_sample as usize).max(self.image.first_sample))
                    } else if (center_offset as usize) < self.image.sample_step {
                        // reached the center => keep cursor there
                        self.first_visible_sample_lock = None;
                        Some(
                            (
                                self.current_sample
                                - self.half_req_sample_window
                            ).max(self.image.first_sample)
                        )
                    } else {
                        // cursor in second half of the window
                        if self.current_sample + self.half_req_sample_window
                            < self.image.last_sample
                        {   // the target sample window doesn't exceed the end
                            // of the rendered waveform yet
                            // => progressively get cursor back to center
                            let previous_cursor = match self.sample_sought {
                                Some(sample_sought) => {
                                    self.sample_sought = None;
                                    sample_sought
                                },
                                None => previous_cursor,
                            };
                            let previous_offset =
                                previous_cursor as i64 - first_visible_sample;
                            let delta_cursor =
                                if self.current_sample >= previous_cursor {
                                    self.current_sample - previous_cursor
                                } else {
                                    previous_cursor - self.current_sample
                                };
                            let next_first_sample =
                                self.current_sample as i64
                                - previous_offset
                                + delta_cursor as i64 / 2;
                            self.first_visible_sample_lock = Some(next_first_sample);

                            Some(self.image.first_sample.max(next_first_sample as usize))
                        } else {
                            // Not enough overhead to get cursor back to center
                            // Follow toward the last sample, but keep
                            // the constraint in case more samples are added
                            // afterward
                            let next_first_sample =
                                if self.image.sample_window >= self.req_sample_window {
                                    // buffer window is larger than req_sample_window
                                    // set last buffer to the right
                                    self.image.last_sample - self.req_sample_window
                                } else {
                                    // buffer window is smaller than req_sample_window
                                    // set first sample to the left
                                    self.image.first_sample
                                };

                            self.first_visible_sample_lock = Some(next_first_sample as i64);
                            Some(next_first_sample)
                        }
                    }
                } else if self.current_sample + self.half_req_sample_window <= self.image.last_sample {
                    // current sample fits in the first half of the window with last sample further
                    if self.current_sample > self.image.first_sample + self.half_req_sample_window {
                        // current sample can be centered (scrolling)
                        Some(self.current_sample - self.half_req_sample_window)
                    } else {
                        // current sample before half of displayable window
                        // set origin to the first sample in the buffer
                        // current sample will be displayed between the origin
                        // and the center
                        Some(self.image.first_sample)
                    }
                } else {
                    // current sample can fit in the second half of the window
                    if self.image.sample_window >= self.req_sample_window {
                        // buffer window is larger than req_sample_window
                        // set last buffer to the right
                        Some(self.image.last_sample - self.req_sample_window)
                    } else {
                        // buffer window is smaller than req_sample_window
                        // set first sample to the left
                        Some(self.image.first_sample)
                    }
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

    // Update rendering conditions
    // return true when an update is required
    pub fn update_condition(&mut self,
        duration: u64,
        width: i32,
        height: i32
    ) -> bool {
        self.req_sample_window = (
            duration as f64 / self.state.sample_duration
        ).round() as usize;
        self.half_req_sample_window = self.req_sample_window / 2;

        self.image.update_dimensions(duration, width, height)
    }

    // Get the waveform as an image in current conditions.
    // This function is to be called as close as possible to
    // the actual presentation of the waveform.
    pub fn get_image(&mut self) -> Option<(&cairo::ImageSurface, f64, Option<f64>)> {
                                        // (image, x_offset, current_x_opt)
        if !self.is_seeking {
            match self.get_first_visible_sample() {
                Some(first_visible_sample) => {
                    let first_visible_sample_f = first_visible_sample as f64;
                    Some((
                        self.image.get_image(),
                        (first_visible_sample_f - self.image.first_sample as f64)
                            / self.image.sample_step_f, // x_offset
                        Some((self.current_sample as f64 - first_visible_sample_f)
                            / self.image.sample_step_f), // current_x_opt
                    ))
                },
                None => None,
            }
        } else {
            // seeking
            match self.first_visible_sample_lock {
                Some(first_visible_sample) => {
                    // first sample is locked
                    // => can draw previous samples window and
                    // move cursor to the position sought
                    let sample_sought = self.sample_sought
                        .expect("WaveformBuffer no sought position while updating conditions in seeking mode");
                    let current_x_opt =
                        if sample_sought > first_visible_sample as usize
                        && sample_sought
                            < first_visible_sample as usize + self.req_sample_window
                        {
                            Some(
                                (sample_sought - first_visible_sample as usize) as f64
                                    / self.image.sample_step_f
                            )
                        } else {
                            None
                        };
                    Some((
                        self.image.get_image(),
                        (first_visible_sample as f64 - self.image.first_sample as f64)
                            / self.image.sample_step_f, // x_offset
                        current_x_opt,
                    ))
                },
                None => None, // no lock, don't draw in order to avoid garbage
            }
        }
    }
}

impl SampleExtractor for WaveformBuffer {
    fn as_mut_any(&mut self) -> &mut Any {
        self
    }

    fn get_extraction_state(&self) -> &SampleExtractionState {
        &self.state
    }

    fn get_extraction_state_mut(&mut self) -> &mut SampleExtractionState {
        &mut self.state
    }

    fn get_first_sample(&self) -> usize {
        self.image.first_sample
    }

    fn cleanup(&mut self) {
        // clear for reuse
        self.cleanup_state();
        self.image.cleanup();

        self.is_seeking = false;
        self.current_sample = 0;
        self.first_visible_sample_lock = None;
        self.sample_sought = None;

        self.was_exposed = false;
        self.req_sample_window = 0;
        self.half_req_sample_window = 0;
    }

    fn update_concrete_state(&mut self, other: &mut Box<SampleExtractor>) {
        let other = other.as_mut_any().downcast_mut::<WaveformBuffer>()
            .expect("WaveformBuffer.update_concrete_state: unable to downcast other ");
        if other.was_exposed {
            self.is_seeking = other.is_seeking;
            self.first_visible_sample_lock = other.first_visible_sample_lock;
            self.sample_sought = other.sample_sought;
            self.current_sample = other.current_sample;

            self.req_sample_window = other.req_sample_window;
            self.half_req_sample_window = other.half_req_sample_window;

            self.image.update_from_other(&other.image);

            other.clear_exposed_status();
        } // else: other has nothing new
    }

    // This is the entry point for the update of the waveform.
    // This function tries to merge the samples added to the AudioBuffer
    // since last extraction and adapt to the evolvin conditions of
    // the playback position and target rendering dimensions and
    // resolution.
    fn extract_samples(&mut self, audio_buffer: &AudioBuffer) {
        if self.state.sample_duration == 0f64 {
            self.state.sample_duration = audio_buffer.sample_duration;
        }

        if self.req_sample_window == 0 {
            // conditions not defined yet
            return;
        }

        let (first_sample, last_sample) = {
            if self.is_seeking {
                // was seeking but since we are receiving an new
                // buffer, it means that sync is done
                //  => force current sample query
                self.current_sample = self.query_current_sample();
            }

            if self.first_visible_sample_lock.is_some()
            && (
                self.current_sample < self.image.first_sample
                || self.current_sample >= self.image.last_sample
            )
            {   // seeking out of previous window
                // clear previous seeking constraint in current window
                self.first_visible_sample_lock = None;
                self.sample_sought = None;
            } // else still in current window => don't worry

            // see how buffers can merge
            let (first_sample, last_sample) =
                if !self.is_seeking {
                    // not seeking => expose the whole buffer
                    (
                        audio_buffer.first_sample,
                        audio_buffer.last_sample
                    )
                } else {
                    // seeking
                    self.is_seeking = false;

                    if audio_buffer.first_sample >= self.image.first_sample
                    && audio_buffer.first_sample < self.image.last_sample
                    {   // new origin further than current
                        // but buffer can be merged with current waveform
                        // or is contained in current waveform
                        //println!("AudioWaveform seeking: can merge to the right");
                        (
                            self.image.first_sample,
                            audio_buffer.last_sample.max(self.image.last_sample)
                        )
                    } else if audio_buffer.first_sample < self.image.first_sample
                    && audio_buffer.last_sample >= self.image.first_sample
                    {   // current waveform overlaps with buffer on its left
                        // or is contained in buffer
                        //println!("AudioWaveform seeking: can merge to the left");
                        (
                            audio_buffer.first_sample,
                            audio_buffer.last_sample.max(self.image.last_sample)
                        )
                    } else {
                        // not able to merge buffer with current waveform
                        //println!("AudioWaveform seeking: not able to merge");
                        (
                            audio_buffer.first_sample,
                            audio_buffer.last_sample
                        )
                    }
                };

            if self.current_sample
                >= first_sample + self.half_req_sample_window
            && self.current_sample + self.half_req_sample_window
                < last_sample
            {
                // nominal case where the position can be centered on screen
                let first_visible_sample =
                    if let Some(first_visible_sample) = self.first_visible_sample_lock {
                        // an in-window seek constraint is pending
                        first_visible_sample as usize
                    } else {
                        self.current_sample - self.half_req_sample_window
                    };
                let last_visible_sample =
                    if !audio_buffer.eos {
                        // Not the end of the stream yet:
                        // attempt to get a larger buffer in order to compensate
                        // for the delay when it will actually be drawn
                        // and for potentiel seek backward
                        last_sample.min(
                            first_visible_sample
                            + self.req_sample_window + self.half_req_sample_window
                        )
                    } else {
                        // Reached the end of the stream
                        // This means that, in case the users doesn't seek,
                        // there won't be any further updates on behalf of
                        // the audio buffer.
                        // => Render the waveform until last sample
                        last_sample
                    };
                (
                    first_visible_sample.max(first_sample),
                    last_visible_sample,
                )
            } else {
                // not enough samples for the requested window
                // around current position
                (
                    first_sample,
                    last_sample,
                )
            }
        };

        self.image.render(
            audio_buffer,
            first_sample,
            last_sample,
            self.state.sample_duration
        );
    }

    // Refresh the waveform in its current sample range
    // and position
    fn refresh(&mut self, audio_buffer: &AudioBuffer) {
        // make sure current is up to date
        self.current_sample = self.query_current_sample();

        let first_sample = self.image.first_sample;
        let last_sample = self.image.last_sample;
        self.image.render(
            audio_buffer,
            first_sample,
            last_sample,
            self.state.sample_duration
        );
    }
}
