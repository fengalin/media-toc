use std::collections::vec_deque::VecDeque;

use super::AudioBuffer;

pub struct WaveformBuffer {
    sample_duration: f64,

    pub current_sample: usize,
    requested_sample_window: usize,
    half_requested_sample_window: usize,
    pub requested_step_duration: u64,

    pub step_duration: f64,
    sample_step: usize,
    pub first_visible_pts: f64,

    pub first_pts: f64,

    samples_offset: usize,
    last_sample: usize,
    pub samples: VecDeque<f64>,
}

impl WaveformBuffer {
    pub fn new() -> Self {
        WaveformBuffer {
            sample_duration: 0f64,

            current_sample: 0,
            requested_sample_window: 0,
            half_requested_sample_window: 0,
            requested_step_duration: 0,

            step_duration: 0f64,
            sample_step: 0,
            first_visible_pts: 0f64,
            first_pts: 0f64,

            samples_offset: 0,
            last_sample: 0,
            samples: VecDeque::new(),
        }
    }

    pub fn update_conditions(&mut self, pts: u64, duration: u64, step_duration: u64) {
        self.set_position(pts);

        self.requested_sample_window = (
            duration as f64 / self.sample_duration
        ).round() as usize;
        self.half_requested_sample_window = self.requested_sample_window / 2;

        self.requested_step_duration = step_duration;

        if !self.samples.is_empty() {
            let first_visible_sample =
                if self.current_sample + self.half_requested_sample_window
                    > self.last_sample
                {
                    if self.samples.len() * self.sample_step > self.requested_sample_window {
                        self.last_sample - self.requested_sample_window
                    }
                    else {
                        self.samples_offset
                    }
                } else if self.current_sample > self.half_requested_sample_window
                    + self.samples_offset
                {
                    self.current_sample - self.half_requested_sample_window
                } else {
                    self.samples_offset
                };

            self.first_visible_pts =
                first_visible_sample as f64 * self.sample_duration;
        }
    }

    pub fn set_position(&mut self, pts: u64) {
        self.current_sample = (
            pts as f64 / self.sample_duration
        ).round() as usize;
    }

    pub fn update_samples(&mut self, audio_buffer: &AudioBuffer) {
        self.sample_duration = audio_buffer.sample_duration;

        if self.requested_sample_window > 0 {
            // use an integer number of samples per step
            let sample_step = (
                self.requested_step_duration as f64
                / self.sample_duration
            ).round();
            let step_duration = sample_step * self.sample_duration;
            let sample_step = sample_step as usize;

            let (first_visible_sample, sample_window) =
                if self.current_sample + self.half_requested_sample_window
                    > audio_buffer.samples_offset + audio_buffer.samples.len()
                {
                    if audio_buffer.samples.len() > self.requested_sample_window {
                        (
                            audio_buffer.samples_offset + audio_buffer.samples.len()
                                - self.requested_sample_window,
                            self.requested_sample_window
                        )
                    }
                    else {
                        (audio_buffer.samples_offset, audio_buffer.samples.len())
                    }
                } else if self.current_sample > self.half_requested_sample_window
                    + audio_buffer.samples_offset
                {
                    // attempt to get a 20% larger buffer in order to compensate
                    // for the delay when it will actually be drawn
                    (
                        self.current_sample - self.half_requested_sample_window,
                        audio_buffer.samples.len().min(
                            self.requested_sample_window + self.requested_sample_window / 20
                        )
                    )
                } else {
                    (audio_buffer.samples_offset, audio_buffer.samples.len())
                };

            // align requested first pts in order to keep a regular
            // offset between redraws. This allows using the same samples
            // for a given requested_step_duration and avoids flickering
            // between redraws
            let first_sample =
                (first_visible_sample / sample_step) * sample_step;

            let last_sample_idx_rel =
                first_sample + sample_window - audio_buffer.samples_offset;

            if sample_step != self.sample_step {
                // resolution has changed or first buffer fill
                self.samples.clear();
                let mut sample_idx_rel = first_sample - audio_buffer.samples_offset;
                while sample_idx_rel < last_sample_idx_rel {
                    self.samples.push_back(audio_buffer.samples[sample_idx_rel]);
                    sample_idx_rel += sample_step;
                }
            } else {
                // remove unused samples
                let new_first_sample_wf_rel =
                    (first_sample - self.samples_offset) / sample_step;
                self.samples.drain(..new_first_sample_wf_rel);

                // add missing samples
                let mut sample_idx_rel =
                    self.last_sample - audio_buffer.samples_offset;
                while sample_idx_rel < last_sample_idx_rel {
                    self.samples.push_back(audio_buffer.samples[sample_idx_rel]);
                    sample_idx_rel += sample_step;
                }
            }

            self.sample_step = sample_step;
            self.step_duration = step_duration;
            self.samples_offset = first_sample;
            self.last_sample = self.samples_offset + self.samples.len() * self.sample_step;
            self.first_pts = first_sample as f64 * self.sample_duration;
            self.first_visible_pts =
                first_visible_sample as f64 * self.sample_duration;
        } // else wait until UI requests something
    }
}
