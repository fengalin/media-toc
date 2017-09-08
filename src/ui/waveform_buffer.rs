extern crate cairo;

use std::any::Any;

use ::media::{AudioBuffer, SAMPLES_NORM};

use ::media::{DoubleSampleExtractor, SamplesExtractor};
use ::media::samples_extractor::SamplesExtractionState;

pub struct DoubleWaveformBuffer {}
impl DoubleWaveformBuffer {
    pub fn new() -> DoubleSampleExtractor {
        DoubleSampleExtractor::new(
            Box::new(WaveformBuffer::new()),
            Box::new(WaveformBuffer::new()),
        )
    }
}

pub struct WaveformBuffer {
    state: SamplesExtractionState,
    buffer_sample_window: usize,

    height: i32,
    pub image_surface: Option<cairo::ImageSurface>,

    pub x_offset: usize,
    pub current_x: usize,
}

impl WaveformBuffer {
    pub fn new() -> Self {
        WaveformBuffer {
            state: SamplesExtractionState::new(),
            buffer_sample_window: 0,

            height: 0,
            image_surface: None,

            x_offset: 0,
            current_x: 0,
        }
    }

    pub fn update_conditions(&mut self,
        pts: u64,
        duration: u64,
        width: i32,
        height: i32,
    )
    {
        let state = &mut self.state;

        self.height = height;

        state.current_sample = (
            pts as f64 / state.sample_duration
        ).round() as usize;

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

        if self.image_surface.is_some() {
            let first_visible_sample =
                if state.eos
                && state.current_sample + state.half_requested_sample_window > state.last_sample {
                    if self.buffer_sample_window > state.requested_sample_window {
                        state.last_sample - state.requested_sample_window
                    } else {
                        state.samples_offset
                    }
                } else {
                    if state.current_sample > state.half_requested_sample_window
                        + state.samples_offset
                    {
                        state.current_sample - state.half_requested_sample_window
                    } else {
                        state.samples_offset
                    }
                };

            self.x_offset = (first_visible_sample - state.samples_offset) / state.sample_step;
            self.current_x = (state.current_sample - first_visible_sample) / state.sample_step;
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
        let buffer_sample_window = last_sample - first_sample;

        // TODO: use 2 prealocated image_surface
        // in order to reuse them when dimensions are kept the same
        let image_surface = cairo::ImageSurface::create(
                cairo::Format::ARgb32,
                (buffer_sample_window / sample_step) as i32,
                self.height
            ).expect("WaveformBuffer: couldn't create image surface in update_extraction");
        let cr = cairo::Context::new(&image_surface);

        let (mut sample_iter, mut x) =
            if self.state.sample_step != sample_step {
                // Resolution has changed or initialization
                // redraw the whole range
                self.state.sample_step = sample_step;

                (
                    audio_buffer.iter(first_sample, last_sample, sample_step),
                    0f64,
                )
            } else {
                // shift previous context
                let previous_image = self.image_surface.take()
                    .expect("WaveformBuffer: no image_surface while updating");

                let sample_step_offset =
                    (first_sample - self.state.samples_offset) / sample_step;
                cr.set_source_surface(
                    &previous_image,
                    -(sample_step_offset as f64),
                    0f64
                );
                cr.paint();

                // prepare to add remaining samples
                (
                    audio_buffer.iter(self.state.last_sample, last_sample, sample_step),
                    (
                        (self.state.last_sample - self.state.samples_offset) / sample_step
                        - sample_step_offset
                    ) as f64,
                )
            };

        if sample_iter.size_hint().0 > 0 {
            // Stroke selected samples
            cr.scale(1f64, self.height as f64 / SAMPLES_NORM);
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

        self.image_surface = Some(image_surface);

        self.state.samples_offset = first_sample;
        self.buffer_sample_window = buffer_sample_window;
        self.state.last_sample = last_sample;
    }
}
