use std::collections::vec_deque::VecDeque;

use super::AudioBuffer;

pub struct WaveformBuffer {
    pub current_pts: u64,
    pub requested_duration: u64,
    pub requested_step_duration: u64,
    pub step_duration: u64,
    pub first_visible_pts: u64,

    pub first_pts: u64,
    pub last_pts: u64,
    pub duration: u64,

    samples_offset: usize,
    pub samples: VecDeque<f64>,
}

impl WaveformBuffer {
    pub fn new() -> Self {
        WaveformBuffer {
            current_pts: 0,
            requested_duration: 0,
            requested_step_duration: 0,
            step_duration: 0,
            first_visible_pts: 0,

            first_pts: 0,
            last_pts: 0,
            duration: 0,

            samples_offset: 0,
            samples: VecDeque::new(),
        }
    }

    pub fn update_conditions(&mut self, pts: u64, duration: u64, step_duration: u64) {
        self.current_pts = pts;
        self.requested_duration = duration;
        self.requested_step_duration = step_duration;

        let half_requested_duration = self.requested_duration / 2;
        let (first_visible_pts, usable_duration) =
            if self.current_pts + half_requested_duration > self.last_pts {
                if self.duration > self.requested_duration {
                    (
                        self.last_pts - self.requested_duration,
                        self.requested_duration
                    )
                }
                else {
                    (self.first_pts, self.duration)
                }
            } else if self.current_pts > half_requested_duration + self.first_pts {
                (
                    self.current_pts - half_requested_duration,
                    self.duration.min(self.requested_duration)
                )
            } else {
                (self.first_pts, self.duration)
            };

        self.first_visible_pts = first_visible_pts;
        self.duration = usable_duration;
    }

    pub fn set_position(&mut self, pts: u64) {
        self.current_pts = pts;
    }

    pub fn update_samples(&mut self, audio_buffer: &AudioBuffer) {
        if self.requested_duration > 0 {
            // use an integer number of samples per step
            let sample_step =
                self.requested_step_duration / audio_buffer.sample_duration;
            self.step_duration = sample_step * audio_buffer.sample_duration;
            let sample_step = sample_step as usize;

            let half_requested_duration = self.requested_duration / 2;
            let (first_visible_pts, duration) =
                if self.current_pts + half_requested_duration > audio_buffer.last_pts {
                    if audio_buffer.duration > self.requested_duration {
                        (
                            audio_buffer.last_pts - self.requested_duration,
                            self.requested_duration
                        )
                    }
                    else {
                        (audio_buffer.first_pts, audio_buffer.duration)
                    }
                } else if self.current_pts > half_requested_duration + audio_buffer.first_pts {
                    // attempt to get a 20% larger buffer in order to compensate
                    // for the delay when it will actually be drawn
                    (
                        self.current_pts - half_requested_duration,
                        audio_buffer.duration.min(
                            self.requested_duration + self.requested_duration / 20
                        )
                    )
                } else {
                    (audio_buffer.first_pts, audio_buffer.duration)
                };

            self.first_visible_pts = first_visible_pts;

            // align requested first pts in order to keep a regular
            // offset between redraws. This allows using the same samples
            // for a given requested_step_duration and avoids flickering
            // between redraws
            self.first_pts =
                (self.first_visible_pts / self.step_duration)
                * self.step_duration;

            self.last_pts = self.first_visible_pts + duration;
            self.duration = self.last_pts - self.first_pts;

            self.samples_offset = (
                self.first_pts / audio_buffer.sample_duration
            ) as usize;

            let last_sample_idx_rel = (
                self.last_pts / audio_buffer.sample_duration
            ) as usize - audio_buffer.samples_offset;
            let last_sample_idx_rel =
                last_sample_idx_rel.min(audio_buffer.samples.len());

            // TODO: implement the strategy to minimize copies
            // by deleting the unused buffer in the front
            // and adding the missing buffer in the back

            self.samples.clear();
            let mut sample_idx_rel = self.samples_offset - audio_buffer.samples_offset;
            while sample_idx_rel < last_sample_idx_rel {
                self.samples.push_back(audio_buffer.samples[sample_idx_rel]);
                sample_idx_rel += sample_step;
            }
        } // else wait until UI requests something
    }
}
