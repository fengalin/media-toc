use std::any::Any;

use super::AudioBuffer;

pub struct SamplesExtractionState {
    pub sample_duration: f64,

    pub current_sample: usize,
    pub samples_offset: usize,
    pub last_sample: usize,

    pub requested_sample_window: usize,
    pub half_requested_sample_window: usize,
    pub requested_step_duration: u64,

    pub sample_step: usize,

    pub eos: bool,
}

impl SamplesExtractionState {
    pub fn new() -> Self {
        SamplesExtractionState {
            sample_duration: 0f64,

            current_sample: 0,
            samples_offset: 0,
            last_sample: 0,

            requested_sample_window: 0,
            half_requested_sample_window: 0,
            requested_step_duration: 0,

            sample_step: 0,
            eos: false,
        }
    }
}

pub trait SamplesExtractor: Send {
    fn as_mut_any(&mut self) -> &mut Any;
    fn get_extraction_state(&self) -> &SamplesExtractionState;
    fn get_extraction_state_mut(&mut self) -> &mut SamplesExtractionState;

    fn can_extract(&self) -> bool;

    fn update_extraction(&mut self,
        audio_buffer: &AudioBuffer,
        first_sample: usize,
        last_sample: usize,
        sample_step: usize,
    );

    fn get_sample_offset(&self) -> usize {
        let state = self.get_extraction_state();
        state.samples_offset
    }

    fn handle_eos(&mut self) {
        let state = self.get_extraction_state_mut();
        state.eos = true;
    }

    fn extract_samples(&mut self, audio_buffer: &AudioBuffer) {
        let (first_visible_sample, sample_window, sample_step) = {
            let can_extract = self.can_extract();

            let state = self.get_extraction_state_mut();
            state.sample_duration = audio_buffer.sample_duration;

            if !can_extract {
                return;
            }

            // use an integer number of samples per step
            let sample_step = (
                state.requested_step_duration as f64
                / state.sample_duration
            ).round();
            let sample_step = sample_step as usize;

            if state.eos {
                if audio_buffer.samples.len() > state.requested_sample_window {
                    let first_visible_sample =
                        state.current_sample - state.half_requested_sample_window;
                    (
                        first_visible_sample,
                        audio_buffer.samples_offset + audio_buffer.samples.len()
                            - first_visible_sample,
                        sample_step
                    )
                } else {
                    (
                        audio_buffer.samples_offset,
                        audio_buffer.samples.len(),
                        sample_step
                    )
                }
            } else if state.current_sample > state.half_requested_sample_window
                + audio_buffer.samples_offset
            {
                // attempt to get a larger buffer in order to compensate
                // for the delay when it will actually be drawn
                let first_visible_sample =
                    state.current_sample - state.half_requested_sample_window;
                let available_duration =
                    audio_buffer.samples_offset + audio_buffer.samples.len()
                    - first_visible_sample;
                (
                    first_visible_sample,
                    available_duration.min(
                        state.requested_sample_window + state.half_requested_sample_window
                    ),
                    sample_step
                )
            } else {
                (
                    audio_buffer.samples_offset,
                    audio_buffer.samples.len(),
                    sample_step
                )
            }
        };

        // align requested first pts in order to keep a regular
        // offset between redraws. This allows using the same samples
        // for a given requested_step_duration and avoiding flickering
        // between redraws
        let first_sample =
            (first_visible_sample / sample_step) * sample_step;

        self.update_extraction(
            audio_buffer,
            first_sample,
            first_sample + sample_window,
            sample_step
        );
    }
}
