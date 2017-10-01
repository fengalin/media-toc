extern crate cairo;

use std::any::Any;

use std::boxed::Box;

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
    previous_sample: usize,
    current_sample: usize,
    first_visible_sample_lock: Option<i64>,
    sought_sample: Option<usize>,

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
            previous_sample: 0,
            current_sample: 0,
            first_visible_sample_lock: None,
            sought_sample: None,

            was_exposed: false,
            req_sample_window: 0,
            half_req_sample_window: 0,
        }
    }

    fn update_current_sample(&mut self) {
        let current_sample = self.query_current_sample();
        if self.previous_sample != current_sample {
            self.previous_sample = self.current_sample;
            self.current_sample = current_sample;
        } // else don't override self.previous_sample
    }

    pub fn clear_exposed_status(&mut self) {
        self.was_exposed = false;
    }

    pub fn seek(&mut self, position: u64, is_playing: bool) {
        let sought_sample = (position as f64 / self.state.sample_duration) as usize;
        if is_playing {
            // stream is playing => let the cursor move from current position
            // to the sought position if possible
            self.first_visible_sample_lock =
                match self.get_first_visible_sample() {
                    Some(first_visible_sample) => {
                        if first_visible_sample <= sought_sample
                        && sought_sample < first_visible_sample + self.req_sample_window
                        {   // Sought sample is included in current window
                            // => lock the first sample so that the cursor appears
                            // at the sought position without abrutely scrolling
                            // the waveform.
                            Some(first_visible_sample as i64)
                        } else {
                            // Sought sample not in current window
                            // => scroll directly to new position
                            None
                        }
                    },
                    None => None,
                };
            self.is_seeking = true;
        } else {
            // not playing => move directly to the sought position
            self.first_visible_sample_lock = None;
            self.is_seeking = false;
        }

        self.sought_sample = Some(sought_sample);
        self.was_exposed = true;
    }

    // Get the stream position from the in-window x coordinate.
    pub fn get_position(&mut self, x: f64) -> Option<u64> {
        match self.get_first_visible_sample() {
            Some(first_visible_sample) => {
                let sought_sample = first_visible_sample as f64 + x * self.image.sample_step_f;
                Some((sought_sample * self.state.sample_duration) as u64)
            },
            None => None,
        }
    }

    // Update to current position and compute the first
    // sample to present for display.
    fn get_first_visible_sample(&mut self) -> Option<usize> {
        if self.image.is_ready() {
            self.was_exposed = true;
            self.update_current_sample();

            if self.current_sample >= self.image.first_sample {
                // current sample appears after first buffer sample
                if let Some(first_visible_sample_lock) = self.first_visible_sample_lock {
                    // There is a position lock constraint
                    // (resulting from an in window seek).
                    let (cursor_sample, previous_sample) =
                        match self.sought_sample {
                            Some(sought_sample) => {
                                if !self.is_seeking {
                                    // not seeking anymore
                                    // => follow current position
                                    self.sought_sample = None;
                                }
                                (sought_sample, sought_sample)
                            },
                            None =>
                                (self.current_sample, self.previous_sample),
                        };
                    let center_offset = cursor_sample as i64
                        - self.half_req_sample_window as i64
                        - first_visible_sample_lock;
                    if center_offset < -(self.image.sample_step as i64) {
                        // cursor in first half of the window
                        // keep origin on the first sample upon seek
                        // this is in case we move toward the center
                        Some((first_visible_sample_lock as usize).max(self.image.first_sample))
                    } else if (center_offset as usize) < 2 * self.image.sample_step {
                        // reached the center => keep cursor there
                        self.first_visible_sample_lock = None;
                        self.sought_sample = None;
                        Some(
                            self.image.first_sample.max(
                                cursor_sample - self.half_req_sample_window
                            )
                        )
                    } else {
                        // cursor in second half of the window
                        if cursor_sample + self.half_req_sample_window
                            < self.image.last_sample
                        {   // the target sample window doesn't exceed the end
                            // of the rendered waveform yet
                            // => progressively get cursor back to center
                            let previous_offset =
                                previous_sample as i64 - first_visible_sample_lock;
                            let delta_cursor =
                                cursor_sample as i64 - previous_sample as i64;
                            let next_first_sample =
                                (self.image.first_sample as i64).max(
                                    cursor_sample as i64 - previous_offset
                                    + delta_cursor
                                );

                            self.first_visible_sample_lock = Some(next_first_sample);
                            Some(next_first_sample as usize)
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
    pub fn update_conditions(&mut self, duration: u64, width: i32, height: i32) {
        let req_sample_window = (
            duration as f64 / self.state.sample_duration
        ).round() as usize;

        if req_sample_window != self.req_sample_window {
            // sample window has changed => zoom
            if let Some(first_visible_sample) = self.first_visible_sample_lock {
                // There is a first visible sample constraint
                // => adapt it to match the new zoom
                let cursor_sample =
                    match self.sought_sample {
                        Some(sought_sample) => sought_sample,
                        None => self.current_sample,
                    };
                let first_visible_sample = first_visible_sample + (
                    (cursor_sample as i64 - first_visible_sample) as f64
                        * (
                            1f64
                            - req_sample_window as f64 / self.req_sample_window as f64
                        )
                    ) as i64;
                self.first_visible_sample_lock = Some(first_visible_sample);
            }
        }

        self.req_sample_window = req_sample_window;
        self.half_req_sample_window = self.req_sample_window / 2;

        self.image.update_dimensions(duration, width, height);
    }

    // Get the waveform as an image in current conditions.
    // This function is to be called as close as possible to
    // the actual presentation of the waveform.
    pub fn get_image(&mut self) -> Option<(&cairo::ImageSurface, f64, Option<f64>)> {
                                        // (image, x_offset, current_x_opt)
        match self.get_first_visible_sample() {
            Some(first_visible_sample) => {
                let first_visible_sample_f = first_visible_sample as f64;
                let current_x_opt =
                    if self.current_sample >= first_visible_sample
                    && self.current_sample
                        <= first_visible_sample + self.req_sample_window
                    {
                        Some(
                            (self.current_sample as f64 - first_visible_sample_f)
                            / self.image.sample_step_f
                        )
                    } else {
                        None
                    };

                Some((
                    self.image.get_image(),
                    (first_visible_sample_f - self.image.first_sample as f64)
                        / self.image.sample_step_f, // x_offset
                    current_x_opt,
                ))
            },
            None => None,
        }
    }

    fn get_sample_range(&mut self, audio_buffer: &AudioBuffer) -> (usize, usize) {
        if !self.is_seeking {
            // not seeking => expose the whole buffer
            (
                audio_buffer.first_sample,
                audio_buffer.last_sample
            )
        } else {
            // seeking
            self.is_seeking = false;

            if audio_buffer.first_sample <= self.image.first_sample
            && audio_buffer.last_sample >= self.image.last_sample {
                // waveform contained in buffer
                //println!("AudioWaveform seeking: waveform contained in buffer");
                (
                    audio_buffer.first_sample,
                    audio_buffer.last_sample
                )
            } else if audio_buffer.first_sample >= self.image.first_sample
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
        }
    }
}

// This is a container to pass conditions via the refresh
// function of the SampleExtractor trait
#[derive(Clone)]
pub struct WaveformConditions {
    pub duration: u64,
    pub width: i32,
    pub height: i32
}

impl WaveformConditions {
    pub fn new(duration: u64, width: i32, height: i32) -> Self {
        WaveformConditions {
            duration: duration,
            width: width,
            height: height
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
        self.previous_sample = 0;
        self.current_sample = 0;
        self.first_visible_sample_lock = None;
        self.sought_sample = None;

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
            self.sought_sample = other.sought_sample;
            self.previous_sample = other.previous_sample;
            self.current_sample = other.current_sample;

            self.req_sample_window = other.req_sample_window;
            self.half_req_sample_window = other.half_req_sample_window;

            self.image.update_from_other(&other.image);

            other.clear_exposed_status();
        } // else: other has nothing new
    }

    // This is the entry point for the update of the waveform.
    // This function tries to merge the samples added to the AudioBuffer
    // since last extraction and adapts to the evolving conditions of
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

        // Get the available sample range considering both
        // the waveform image and the AudioBuffer
        let (first_sample, last_sample) = self.get_sample_range(&audio_buffer);

        if self.is_seeking {
            // was seeking but since we are receiving a new
            // buffer, it means that sync is done
            //  => force current sample update
            self.update_current_sample();
        }

        let (first_sample_to_extract, last_sample_to_extract) =
            if self.current_sample
                >= first_sample + self.half_req_sample_window
            && self.current_sample + self.half_req_sample_window
                < last_sample
            {   // Nominal case where the position can be centered on screen.
                // Don't worry about possible first sample constraint here
                // it will be dealt with when the image is actually drawn on screen.
                let first_to_extract =
                    if let Some(first_visible_sample) = self.first_visible_sample_lock {
                        // an in-window seek constraint is pending
                        first_visible_sample as usize
                    } else if self.current_sample > first_sample + self.half_req_sample_window {
                        self.current_sample - self.half_req_sample_window
                    } else {
                        first_sample
                    };

                let last_to_extract =
                    if !audio_buffer.eos {
                        // Not the end of the stream yet:
                        // attempt to get a larger buffer in order to compensate
                        // for the delay when it will actually be drawn
                        // and for potential seek forward without lock
                        last_sample.min(
                            first_to_extract
                            + self.req_sample_window + self.half_req_sample_window
                        )
                    } else {
                        // Reached the end of the stream
                        // This means that, in case the user doesn't seek,
                        // there won't be any further updates on behalf of
                        // the audio buffer.
                        // => Render the waveform until last sample
                        last_sample
                    };
                (
                    first_to_extract,
                    last_to_extract,
                )
            } else {
                // not enough samples for the requested window
                // around current position
                (
                    first_sample,
                    last_sample,
                )
            };

        self.image.render(
            audio_buffer,
            first_sample_to_extract,
            last_sample_to_extract,
            self.state.sample_duration
        );
    }

    fn refresh(&mut self, audio_buffer: &AudioBuffer) {
        if self.image.is_ready {
            // make sure current is up to date
            self.update_current_sample();

            let (first_sample, last_sample) = self.get_sample_range(audio_buffer);

            // attempt to get an image with a window before current position
            // and half a window after in order to be able to seek
            // and force cursor to center
            let first_sample_to_extract =
                if self.current_sample > first_sample + self.req_sample_window {
                    self.current_sample - self.req_sample_window
                } else {
                    first_sample
                };

            self.image.render(
                audio_buffer,
                first_sample_to_extract,
                last_sample.min(self.current_sample + self.req_sample_window),
                self.state.sample_duration
            );
        } // no need to refresh
    }

    // Refresh the waveform in its current sample range and position
    fn refresh_with_conditions(&mut self, audio_buffer: &AudioBuffer, conditions: Box<Any>) {
        let cndt = conditions.downcast::<WaveformConditions>()
            .expect("WaveformBuffer::refresh conditions is not a WaveformConditions");
        self.update_conditions(cndt.duration, cndt.width, cndt.height);
        self.refresh(&audio_buffer);
    }
}
