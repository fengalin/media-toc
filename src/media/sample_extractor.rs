use gstreamer as gst;
use gstreamer::ClockExt;

use std::any::Any;
use std::boxed::Box;

use super::{AudioBuffer, AudioChannel};

pub struct SampleExtractionState {
    pub sample_duration: u64,
    pub duration_per_1000_samples: f64,
    clock: Option<gst::Clock>,
    segment_start: u64,
    base_time: u64,
}

impl SampleExtractionState {
    pub fn new() -> Self {
        SampleExtractionState {
            sample_duration: 0,
            duration_per_1000_samples: 0f64,
            clock: None,
            segment_start: 0,
            base_time: 0,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        self.sample_duration = 0;
        self.duration_per_1000_samples = 0f64;
        self.clock = None;
        self.segment_start = 0;
        self.base_time = 0;
    }
}

pub trait SampleExtractor: Send {
    fn as_mut_any(&mut self) -> &mut Any;
    fn as_any(&self) -> &Any;
    fn get_extraction_state(&self) -> &SampleExtractionState;
    fn get_extraction_state_mut(&mut self) -> &mut SampleExtractionState;

    fn cleanup(&mut self);

    fn set_clock(&mut self, clock: &gst::Clock) {
        self.get_extraction_state_mut().clock = Some(clock.clone());
    }

    fn new_position(&mut self, segment_start: u64, base_time: u64) {
        let state = self.get_extraction_state_mut();
        state.segment_start = segment_start;
        state.base_time = base_time;
    }

    fn set_channels(&mut self, channels: &[AudioChannel]);

    fn set_sample_duration(&mut self, per_sample: u64, per_1000_samples: f64);

    fn set_conditions(&mut self, conditions: Box<Any>);

    fn get_lower(&self) -> usize;

    fn get_requested_sample_window(&self) -> usize;

    fn switch_to_paused(&mut self);

    // update self with concrete state of other
    // which is expected to be the same concrete type
    // this update is intended at smoothening the specific
    // extraction process by keeping conditions between frames
    fn update_concrete_state(&mut self, other: &mut SampleExtractor);

    fn get_current_sample(&mut self) -> (u64, usize) {
        // (position, sample)
        let state = &mut self.get_extraction_state_mut();
        let position = match state.clock {
            Some(ref clock) => {
                let current_time = clock.get_time().nanoseconds().unwrap();
                current_time - state.base_time + state.segment_start
            }
            None => 0,
        };
        (position, (position / state.sample_duration) as usize)
    }

    // Update the extractions taking account new
    // samples added to the buffer and possibly a
    // different position
    fn extract_samples(&mut self, audio_buffer: &AudioBuffer);

    // Refresh the extractionm in its current sample range
    // and position.
    fn refresh(&mut self, audio_buffer: &AudioBuffer);
}
