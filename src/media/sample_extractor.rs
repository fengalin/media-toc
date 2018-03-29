use gstreamer as gst;
use gstreamer::ElementExtManual;

use std::any::Any;
use std::boxed::Box;

use super::{AudioBuffer, AudioChannel};

pub struct SampleExtractionState {
    pub sample_duration: u64,
    pub duration_per_1000_samples: f64,
    state: gst::State,
    audio_ref: Option<gst::Element>,
    pub time_ref: Option<(u64, u64)>, // (base_time, base_frame_time)
    pub last_pos: u64,
}

impl SampleExtractionState {
    pub fn new() -> Self {
        SampleExtractionState {
            sample_duration: 0,
            duration_per_1000_samples: 0f64,
            state: gst::State::Null,
            audio_ref: None,
            time_ref: None,
            last_pos: 0,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        self.sample_duration = 0;
        self.duration_per_1000_samples = 0f64;
        self.state = gst::State::Null;
        self.audio_ref = None;
        self.time_ref = None;
        self.last_pos = 0;
    }
}

pub trait SampleExtractor: Send {
    fn as_mut_any(&mut self) -> &mut Any;
    fn as_any(&self) -> &Any;
    fn get_extraction_state(&self) -> &SampleExtractionState;
    fn get_extraction_state_mut(&mut self) -> &mut SampleExtractionState;

    fn cleanup(&mut self);

    fn set_state(&mut self, new_state: gst::State) {
        let state = self.get_extraction_state_mut();
        state.state = new_state;
        state.time_ref = None;
    }

    fn set_time_ref(&mut self, audio_ref: &gst::Element) {
        self.get_extraction_state_mut().audio_ref = Some(audio_ref.clone());
    }

    fn new_segment(&mut self) {
        self.get_extraction_state_mut().time_ref = None;
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

    fn get_current_sample(&mut self, frame_time: u64) -> (u64, usize) {
        // (position, sample)
        let state = &mut self.get_extraction_state_mut();
        let position = match state.state {
            gst::State::Playing => {
                if let Some(&(base_time, base_frame_time)) = state.time_ref.as_ref() {
                    (frame_time - base_frame_time) * 1_000 + base_time
                } else {
                    let mut query = gst::Query::new_position(gst::Format::Time);
                    if state.audio_ref.as_ref().unwrap().query(&mut query) {
                        let base_time = query.get_result().get_value() as u64;
                        state.time_ref = Some((base_time, frame_time));
                        base_time
                    } else {
                        state.last_pos
                    }
                }
            }
            gst::State::Paused => {
                let mut query = gst::Query::new_position(gst::Format::Time);
                if state.audio_ref.as_ref().unwrap().query(&mut query) {
                    query.get_result().get_value() as u64
                } else {
                    state.last_pos
                }
            }
            _ => state.last_pos,
        };

        state.last_pos = position;

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
