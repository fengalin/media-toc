use gstreamer as gst;
use gstreamer::ElementExtManual;

use std::any::Any;

use super::{AudioBuffer, AudioChannel};

pub struct SampleExtractionState {
    pub sample_duration: u64,
    pub duration_per_1000_samples: f64,
    state: gst::State,
    audio_ref: Option<gst::Element>,
    pub basetime: Option<(u64, u64)>, // (base_time, base_frame_time)
    pub last_pos: u64,
}

impl SampleExtractionState {
    pub fn new() -> Self {
        SampleExtractionState {
            sample_duration: 0,
            duration_per_1000_samples: 0f64,
            state: gst::State::Null,
            audio_ref: None,
            basetime: None,
            last_pos: 0,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        self.sample_duration = 0;
        self.duration_per_1000_samples = 0f64;
        self.state = gst::State::Null;
        self.audio_ref = None;
        self.basetime = None;
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
        state.basetime = None;
    }

    fn set_time_ref(&mut self, audio_ref: &gst::Element) {
        self.get_extraction_state_mut().audio_ref = Some(audio_ref.clone());
    }

    fn reset_basetime(&mut self) {
        self.get_extraction_state_mut().basetime = None;
    }

    fn new_segment(&mut self) {
        self.get_extraction_state_mut().basetime = None;
        self.seek_complete();
    }

    fn seek_complete(&mut self);

    fn set_channels(&mut self, channels: &[AudioChannel]);

    fn set_sample_duration(&mut self, per_sample: u64, per_1000_samples: f64);

    fn get_lower(&self) -> usize;

    fn get_requested_sample_window(&self) -> Option<usize>;

    fn switch_to_paused(&mut self);

    // update self with concrete state of other
    // which is expected to be the same concrete type
    // this update is intended at smoothening the specific
    // extraction process by keeping conditions between frames
    fn update_concrete_state(&mut self, other: &mut SampleExtractor);

    fn get_current_sample(&mut self, last_frame_time: u64, next_frame_time: u64) -> (u64, usize) {
        // (position, sample)
        let state = &mut self.get_extraction_state_mut();
        let position = match state.state {
            gst::State::Playing => {
                if let Some(&(base_time, base_frame_time)) = state.basetime.as_ref() {
                    (next_frame_time - base_frame_time) * 1_000 + base_time
                } else {
                    let mut query = gst::Query::new_position(gst::Format::Time);
                    if state.audio_ref.as_ref().unwrap().query(&mut query) {
                        // approximate current position as being exactly between last frame
                        // and next frame
                        let current_frame_time = (last_frame_time + next_frame_time) / 2;
                        let base_time = query.get_result().get_value() as u64;
                        state.basetime = Some((base_time, current_frame_time));
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
