extern crate gstreamer as gst;

use std::any::Any;

use std::mem;

use std::sync::{Arc, Mutex};

use super::{AudioBuffer, SampleExtractor};

// The DoubleBuffer is reponsible for ensuring a thread safe double buffering
// mechanism that receives samples from GStreamer, prepares an extraction of
// these samples and presents the most recent extraction to an external
// mechanism (e.g. UI).
// The DoubleBuffer hosts two SampleExtractors and an AudioBuffer:
//   - The SampleExtractor trait is designed to allow the extraction of samples
//   depending on certains conditions as defined in its concrete implementation.
//   - The AudioBuffer manages the container for all the samples received.
// The DoubleBuffer prepares the extraction in the working_buffer and exposes
// the exposed_buffer during that time. When the extration is done, the buffers
// are swapped.
pub struct DoubleAudioBuffer {
    audio_buffer: AudioBuffer,
    exposed_buffer_mtx: Arc<Mutex<Box<SampleExtractor>>>,
    working_buffer: Option<Box<SampleExtractor>>,
    first_sample_to_keep: usize,
}

impl DoubleAudioBuffer {
    // need 2 arguments for new as we can't clone buffers as they are known
    // as trait SampleExtractor
    pub fn new(
        buffer_duration: u64,
        exposed_buffer: Box<SampleExtractor>,
        working_buffer: Box<SampleExtractor>
    ) -> DoubleAudioBuffer {
        DoubleAudioBuffer {
            audio_buffer: AudioBuffer::new(buffer_duration),
            exposed_buffer_mtx: Arc::new(Mutex::new(exposed_buffer)),
            working_buffer: Some(working_buffer),
            first_sample_to_keep: 0,
        }
    }

    // Get a reference on the exposed buffer mutex.
    pub fn get_exposed_buffer_mtx(&self) -> Arc<Mutex<Box<SampleExtractor>>> {
        Arc::clone(&self.exposed_buffer_mtx)
    }

    pub fn cleanup(&mut self) {
        self.audio_buffer.cleanup();

        {
            let exposed_buffer = &mut self.exposed_buffer_mtx.lock()
                .expect("DoubleAudioBuffer: couldn't lock exposed_buffer_mtx while setting audio sink");
            exposed_buffer.cleanup();
        }

        self.working_buffer.as_mut()
            .expect("DoubleAudioBuffer: couldn't get working_buffer while setting audio sink")
            .cleanup();
        self.first_sample_to_keep = 0;
    }

    // Initialize buffer with audio stream capabilities
    // and GStreamer element for position reference
    pub fn set_audio_caps_and_ref(&mut self,
        caps: &gst::Caps,
        audio_ref: &gst::Element
    ) {
        self.audio_buffer.set_caps(caps);

        {
            let exposed_buffer = &mut self.exposed_buffer_mtx.lock()
                .expect("DoubleAudioBuffer: couldn't lock exposed_buffer_mtx while setting audio sink");
            exposed_buffer.set_audio_sink(audio_ref.clone());
        }

        self.working_buffer.as_mut()
            .expect("DoubleAudioBuffer: couldn't get working_buffer while setting audio sink")
            .set_audio_sink(audio_ref.clone());
    }

    pub fn handle_eos(&mut self) {
        self.audio_buffer.handle_eos();
        // extract last samples and swap
        self.extract_samples();
        // update second buffer too
        self.extract_samples();
    }

    pub fn push_gst_sample(&mut self, sample: gst::Sample) {
        // store incoming samples
        self.audio_buffer.push_gst_sample(sample, self.first_sample_to_keep);

        // extract new samples and swap
        self.extract_samples();
    }

    // Update the working extractor with new samples and swap.
    pub fn extract_samples(&mut self) {
        let mut working_buffer = self.working_buffer.take()
            .expect("DoubleAudioBuffer::extract_samples: failed to take working buffer");
        working_buffer.extract_samples(&self.audio_buffer);

        // swap buffers
        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock()
                .expect("DoubleAudioBuffer::extract_samples: failed to lock the exposed buffer");
            // get latest state from the previously exposed buffer
            // in order to smoothen rendering between frames
            working_buffer.update_concrete_state(exposed_buffer_box);
            mem::swap(exposed_buffer_box, &mut working_buffer);
        }

        self.first_sample_to_keep = working_buffer.get_first_sample();

        self.working_buffer = Some(working_buffer);
        // self.working_buffer is now the buffer previously in
        // self.exposed_buffer_mtx
    }

    pub fn refresh(&mut self) {
        // refresh with current conditions
        let mut working_buffer = self.working_buffer.take()
            .expect("DoubleAudioBuffer::refresh: failed to take working buffer");

        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock()
                .expect("DoubleAudioBuffer:::refresh: failed to lock the exposed buffer");
            // get latest state from the previously exposed buffer
            working_buffer.update_concrete_state(exposed_buffer_box);

            // refresh working buffer
            working_buffer.refresh(&self.audio_buffer);

            // swap buffers
            mem::swap(exposed_buffer_box, &mut working_buffer);
        }

        self.first_sample_to_keep = working_buffer.get_first_sample();

        self.working_buffer = Some(working_buffer);
        // self.working_buffer is now the buffer previously in
        // self.exposed_buffer_mtx
    }

    // Refresh the working buffer with the provided conditions and swap.
    // Conditions concrete type must conform to a struct expected
    // by the concrete implementation of the SampleExtractor.
    pub fn refresh_with_conditions<T: Any + Clone>(&mut self, conditions: Box<T>) {
        let mut working_buffer = self.working_buffer.take()
            .expect("DoubleAudioBuffer::refresh: failed to take working buffer");

        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock()
                .expect("DoubleAudioBuffer:::refresh: failed to lock the exposed buffer");
            // get latest state from the previously exposed buffer
            working_buffer.update_concrete_state(exposed_buffer_box);

            // refresh working buffer
            working_buffer.refresh_with_conditions(&self.audio_buffer, conditions.clone());

            // swap buffers
            mem::swap(exposed_buffer_box, &mut working_buffer);
        }

        // working buffer is last exposed buffer
        // refresh with new conditions
        working_buffer.refresh_with_conditions(&self.audio_buffer, conditions);

        self.first_sample_to_keep = working_buffer.get_first_sample();

        self.working_buffer = Some(working_buffer);
        // self.working_buffer is now the buffer previously in
        // self.exposed_buffer_mtx
    }
}
