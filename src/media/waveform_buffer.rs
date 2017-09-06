use std::any::Any;

use std::collections::vec_deque::VecDeque;

use super::AudioBuffer;

use super::SamplesExtractor;
use super::samples_extractor::SamplesExtractionState;

pub struct WaveformBuffer {
    state: SamplesExtractionState,

    pub samples: VecDeque<f64>,

    first_visible_idx: usize,
    last_visible_idx: usize,
    pub first_visible_pts: f64,
}

impl WaveformBuffer {
    pub fn new() -> Self {
        WaveformBuffer {
            state: SamplesExtractionState::new(),

            samples: VecDeque::new(),

            first_visible_idx: 0,
            last_visible_idx: 0,
            first_visible_pts: 0f64,
        }
    }

    pub fn iter(&self) -> Iter {
        Iter::new(self)
    }

    pub fn get_step_duration(&self) -> f64 {
        self.state.sample_step as f64 * self.state.sample_duration
    }

    pub fn update_conditions(&mut self, pts: u64, duration: u64, step_duration: u64) {
        let state = &mut self.state;

        state.current_sample = (
            pts as f64 / state.sample_duration
        ).round() as usize;

        state.requested_sample_window = (
            duration as f64 / state.sample_duration
        ).round() as usize;
        state.half_requested_sample_window = state.requested_sample_window / 2;

        state.requested_step_duration = step_duration;

        if !self.samples.is_empty() {
            let buffer_sample_window = self.samples.len() * state.sample_step;
            let (first_visible_sample, sample_window) =
                if state.eos
                && state.current_sample + state.half_requested_sample_window > state.last_sample {
                    if buffer_sample_window > state.requested_sample_window {
                        (
                            state.last_sample - state.requested_sample_window,
                            state.requested_sample_window
                        )
                    }
                    else {
                        (
                            state.samples_offset,
                            buffer_sample_window
                        )
                    }
                } else {
                    if state.current_sample > state.half_requested_sample_window
                        + state.samples_offset
                    {
                        let first_visible_sample =
                            state.current_sample - state.half_requested_sample_window;
                        let remaining_samples = state.last_sample - first_visible_sample;
                        (
                            first_visible_sample,
                            remaining_samples.min(state.requested_sample_window)
                        )
                    } else {
                        (
                            state.samples_offset,
                            buffer_sample_window.min(state.requested_sample_window)
                        )
                    }
                };

            self.first_visible_idx = (first_visible_sample - state.samples_offset) / state.sample_step;
            self.last_visible_idx = self.first_visible_idx + sample_window / state.sample_step;

            self.first_visible_pts =
                first_visible_sample as f64 * state.sample_duration;
        }
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

    fn update_extraction(&mut self,
        audio_buffer: &AudioBuffer,
        first_sample: usize,
        last_sample: usize,
        sample_step: usize,
    ) {
        if self.state.sample_step != sample_step {
            // resolution has changed or initialization => reset extraction
            self.samples.clear();
            self.state.sample_step = sample_step;
            self.samples.extend(
                audio_buffer.iter(first_sample, last_sample, sample_step)
            );
        } else {
            // incremental update

            // remove no longer necessary samples
            let new_first_sample_idx_rel =
                (first_sample - self.state.samples_offset) / sample_step;
            self.samples.drain(..new_first_sample_idx_rel);

            // add missing samples if any
            if last_sample > self.state.last_sample {
                self.samples.extend(
                    audio_buffer.iter(self.state.last_sample, last_sample, sample_step)
                );
            }
        }

        self.state.samples_offset = first_sample;
        self.state.last_sample = first_sample + self.samples.len() * sample_step;
    }
}

pub struct Iter<'a> {
    buffer: &'a WaveformBuffer,
    idx: usize,
    last: usize,
}

impl<'a> Iter<'a> {
    fn new(buffer: &'a WaveformBuffer) -> Iter<'a> {
        Iter {
            buffer: buffer,
            idx: buffer.first_visible_idx,
            last: buffer.last_visible_idx,
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a f64;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.last {
            return None;
        }

        let item = self.buffer.samples.get(self.idx);
        self.idx += 1;

        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.idx == self.last {
            return (0, Some(0));
        }

        let remaining = self.last - self.idx;

        (remaining, Some(remaining))
    }
}
