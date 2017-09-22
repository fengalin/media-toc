extern crate gstreamer as gst;
use gstreamer::{ElementExtManual, QueryView};

use std::any::Any;

use std::mem;

use std::sync::{Arc, Mutex};

use super::AudioBuffer;

// DoubleSampleExtractor hosts two SampleExtractors
// that can be swapped to implement a double buffer mechanism
// which selects a subset of samples depending on external
// conditions
pub struct DoubleSampleExtractor {
    pub exposed_buffer_mtx: Arc<Mutex<Box<SamplesExtractor>>>,
    working_buffer: Option<Box<SamplesExtractor>>,
}

impl DoubleSampleExtractor {
    // need 2 arguments for new as we can't clone buffers as they are known
    // as trait SamplesExtractor
    pub fn new(
        exposed_buffer: Arc<Mutex<Box<SamplesExtractor>>>,
        working_buffer: Box<SamplesExtractor>
    ) -> DoubleSampleExtractor {
        DoubleSampleExtractor {
            exposed_buffer_mtx: exposed_buffer,
            working_buffer: Some(working_buffer),
        }
    }

    pub fn set_audio_sink(&mut self, audio_sink: &gst::Element) {
        {
            let exposed_buffer = &mut self.exposed_buffer_mtx.lock()
                .expect("Couldn't lock exposed_buffer_mtx while setting audio sink");
            exposed_buffer.set_audio_sink(audio_sink.clone());
        }
        self.working_buffer.as_mut()
            .expect("Couldn't get working_buffer while setting audio sink")
            .set_audio_sink(audio_sink.clone());
    }

    pub fn get_first_sample(&self) -> usize {
        self.working_buffer.as_ref().unwrap().get_first_sample()
    }

    pub fn extract_samples(&mut self, audio_buffer: &AudioBuffer, first_sample_changed: bool) {
        let mut working_buffer = self.working_buffer.take()
            .expect("DoubleSampleExtractor: failed to take working buffer while updating");
        if first_sample_changed {
            working_buffer.set_first_sample_changed();
        }
        working_buffer.extract_samples(audio_buffer);

        // swap buffers
        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock()
                .expect("DoubleSampleExtractor: failed to lock the exposed buffer for swap");
            // get latest conditions from the previously exposed buffer
            // in order to smoothen rendering between frames
            working_buffer.update_concrete_state(exposed_buffer_box);
            mem::swap(exposed_buffer_box, &mut working_buffer);
        }

        // also set first_sample_changed flag for previously exposed buffer
        // in preparation for next samples extraction
        if first_sample_changed {
            working_buffer.set_first_sample_changed();
        }
        self.working_buffer = Some(working_buffer);
        // self.working_buffer is now the buffer previously in
        // self.exposed_buffer_mtx
    }
}

unsafe impl Sync for DoubleSampleExtractor {}

pub struct SamplesExtractionState {
    pub sample_duration: f64,
    audio_sink: Option<gst::Element>,
    position_query: gst::Query,
}

impl SamplesExtractionState {
    pub fn new() -> Self {
        SamplesExtractionState {
            sample_duration: 0f64,
            audio_sink: None,
            position_query: gst::Query::new_position(gst::Format::Time),
        }
    }

    pub fn set_audio_sink(&mut self, audio_sink: gst::Element) {
        self.audio_sink = Some(audio_sink);
    }
}

pub trait SamplesExtractor: Send {
    fn as_mut_any(&mut self) -> &mut Any;
    fn get_extraction_state(&self) -> &SamplesExtractionState;
    fn get_extraction_state_mut(&mut self) -> &mut SamplesExtractionState;

    fn cleanup_state(&mut self) {
        // clear for reuse
        let state = self.get_extraction_state_mut();
        state.sample_duration = 0f64;
        state.audio_sink = None;
    }

    fn set_audio_sink(&mut self, audio_sink: gst::Element) {
        self.get_extraction_state_mut().set_audio_sink(audio_sink);
    }

    fn set_first_sample_changed(&mut self);
    fn get_first_sample(&self) -> usize;

    // update self with concrete state of other
    // which is expected to be the same concrete type
    // this update is intended at smoothening the specific
    // extraction process by keeping conditions between frames
    fn update_concrete_state(&mut self, other: &mut Box<SamplesExtractor>);

    fn query_current_sample(&mut self) -> usize {
        let state = &mut self.get_extraction_state_mut();
        state.audio_sink.as_ref()
            .expect("DoubleSampleExtractor: no audio ref while querying position")
            .query(state.position_query.get_mut().unwrap());
        (
            match state.position_query.view() {
                QueryView::Position(ref position) => position.get().1 as f64,
                _ => unreachable!(),
            } / state.sample_duration
        ).round() as usize
    }

    fn extract_samples(&mut self, audio_buffer: &AudioBuffer);
}
