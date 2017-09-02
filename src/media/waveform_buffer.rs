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
    first_visible_idx: usize,
    last_visible_idx: usize,
    pub first_visible_pts: f64,

    pub first_pts: f64,
    pub eos: bool,

    pub samples_offset: usize,
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
            first_visible_idx: 0,
            last_visible_idx: 0,
            first_visible_pts: 0f64,
            first_pts: 0f64,
            eos: false,

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
            let buffer_sample_window = self.samples.len() * self.sample_step;
            let (first_visible_sample, sample_window) =
                if self.eos
                && self.current_sample + self.half_requested_sample_window > self.last_sample {
                    if buffer_sample_window > self.requested_sample_window {
                        (
                            self.last_sample - self.requested_sample_window,
                            self.requested_sample_window
                        )
                    }
                    else {
                        (
                            self.samples_offset,
                            buffer_sample_window
                        )
                    }
                } else {
                    if self.current_sample > self.half_requested_sample_window
                        + self.samples_offset
                    {
                        let first_visible_sample =
                            self.current_sample - self.half_requested_sample_window;
                        let remaining_samples = self.last_sample - first_visible_sample;
                        (
                            first_visible_sample,
                            remaining_samples.min(self.requested_sample_window)
                        )
                    } else {
                        (
                            self.samples_offset,
                            buffer_sample_window.min(self.requested_sample_window)
                        )
                    }
                };

            self.first_visible_idx = (first_visible_sample - self.samples_offset) / self.sample_step;
            self.last_visible_idx = self.first_visible_idx + sample_window / self.sample_step;

            self.first_visible_pts =
                first_visible_sample as f64 * self.sample_duration;
        }
    }

    pub fn iter(&self) -> Iter {
        Iter::new(self)
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
                if self.eos {
                    if audio_buffer.samples.len() > self.requested_sample_window {
                        let first_visible_sample =
                            self.current_sample - self.half_requested_sample_window;
                        (
                            first_visible_sample,
                            audio_buffer.samples.len().min(
                                audio_buffer.samples_offset + audio_buffer.samples.len()
                                - first_visible_sample
                            )
                        )
                    }
                    else {
                        (audio_buffer.samples_offset, audio_buffer.samples.len())
                    }
                } else if self.current_sample > self.half_requested_sample_window
                    + audio_buffer.samples_offset
                {
                    // attempt to get a larger buffer in order to compensate
                    // for the delay when it will actually be drawn
                    let first_visible_sample =
                        self.current_sample - self.half_requested_sample_window;
                    let available_duration =
                        audio_buffer.samples_offset + audio_buffer.samples.len()
                        - first_visible_sample;
                    (
                        first_visible_sample,
                        available_duration.min(
                            self.requested_sample_window + self.half_requested_sample_window
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

            self.first_visible_idx = (first_visible_sample - self.samples_offset) / self.sample_step;
            self.last_visible_idx = self.samples.len();
        } // else wait until UI requests something
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
