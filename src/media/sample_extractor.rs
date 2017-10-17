extern crate gstreamer as gst;
use gstreamer::{ElementExtManual, QueryView};

use std::any::Any;

use std::boxed::Box;

use super::{AudioBuffer, AudioChannel};

pub struct SampleExtractionState {
    pub sample_duration: u64,
    pub duration_per_1000_samples: f64,
    audio_sink: Option<gst::Element>,
    position_query: gst::Query,
}

impl SampleExtractionState {
    pub fn new() -> Self {
        SampleExtractionState {
            sample_duration: 0,
            duration_per_1000_samples: 0f64,
            audio_sink: None,
            position_query: gst::Query::new_position(gst::Format::Time),
        }
    }

    pub fn set_audio_sink(&mut self, audio_sink: gst::Element) {
        self.audio_sink = Some(audio_sink);
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        self.sample_duration = 0;
        self.duration_per_1000_samples = 0f64;
        self.audio_sink = None;
    }
}

pub trait SampleExtractor: Send {
    fn as_mut_any(&mut self) -> &mut Any;
    fn get_extraction_state(&self) -> &SampleExtractionState;
    fn get_extraction_state_mut(&mut self) -> &mut SampleExtractionState;

    fn cleanup(&mut self);

    fn set_audio_sink(&mut self, audio_sink: gst::Element) {
        self.get_extraction_state_mut().set_audio_sink(audio_sink);
    }

    fn set_channels(&mut self, channels: &[AudioChannel]);

    fn set_sample_duration(&mut self, per_sample: u64, per_1000_samples: f64);

    fn get_lower(&self) -> usize;

    // update self with concrete state of other
    // which is expected to be the same concrete type
    // this update is intended at smoothening the specific
    // extraction process by keeping conditions between frames
    fn update_concrete_state(&mut self, other: &mut Box<SampleExtractor>);

    fn query_current_sample(&mut self) -> (u64, usize) { // (position, sample)
        let state = &mut self.get_extraction_state_mut();
        state.audio_sink.as_ref()
            .expect("DoubleSampleExtractor: no audio ref while querying position")
            .query(state.position_query.get_mut().unwrap());
        let position =
            match state.position_query.view() {
                QueryView::Position(ref position) => position.get().1 as u64,
                _ => unreachable!(),
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

    // Refresh the extractionm in its current sample range
    // and position but with new conditions. E.g. change scale
    fn refresh_with_conditions(&mut self, audio_buffer: &AudioBuffer, conditions: Box<Any>);
}
