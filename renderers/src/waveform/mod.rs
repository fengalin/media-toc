pub mod image;
pub mod renderer;

use media::SampleIndexRange;
use metadata::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub struct Dimensions {
    pub(super) sample_duration: Duration,
    pub(super) sample_step: SampleIndexRange,
    pub(super) sample_step_f: f64,

    pub(super) x_step_f: f64,
    pub(super) x_step: usize,

    pub(super) req_sample_window: SampleIndexRange,
    pub(super) half_req_sample_window: SampleIndexRange,
    pub(super) quarter_req_sample_window: SampleIndexRange,

    pub(super) force_redraw_1: bool,
    pub(super) force_redraw_2: bool,

    pub(super) req_width: i32,
    pub(super) req_width_f: f64,
    pub(super) req_height: i32,

    pub(super) duration_per_1000_samples: Duration,
    pub(super) req_duration_per_1000px: Duration,
}

impl Dimensions {
    pub(super) fn reset(&mut self) {
        *self = Default::default();
    }

    pub(super) fn reset_sample_conditions(&mut self) {
        self.sample_duration = Default::default();
        self.sample_step = Default::default();
        self.sample_step_f = 0f64;
        self.x_step_f = 0f64;
        self.x_step = 0;
        self.force_redraw_1 = true;
        self.force_redraw_2 = true;
    }
}
