use gstreamer as gst;
use gstreamer_audio as gst_audio;

use log::info;

use smallvec::SmallVec;

use std::mem;

use std::sync::{Arc, Mutex};

use super::{AudioBuffer, AudioChannel, SampleExtractor, INLINE_CHANNELS, QUEUE_SIZE_NS};

const EXTRACTION_THRESHOLD: usize = 1024;

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
    state: gst::State,
    audio_buffer: AudioBuffer,
    samples_since_last_extract: usize,
    exposed_buffer_mtx: Arc<Mutex<Box<dyn SampleExtractor>>>,
    working_buffer: Option<Box<dyn SampleExtractor>>,
    lower_to_keep: usize,
    sample_gauge: Option<usize>,
    sample_window: Option<usize>,
    max_sample_window: usize,
    can_handle_eos: bool, // accept / ignore eos (required for range seeks handling)
    has_new_position: bool,
}

impl DoubleAudioBuffer {
    // need 2 arguments for new as we can't clone buffers as they are known
    // as trait SampleExtractor
    pub fn new(
        buffer_duration: u64,
        exposed_buffer: Box<dyn SampleExtractor>,
        working_buffer: Box<dyn SampleExtractor>,
    ) -> DoubleAudioBuffer {
        DoubleAudioBuffer {
            state: gst::State::Null,
            audio_buffer: AudioBuffer::new(buffer_duration),
            samples_since_last_extract: 0,
            exposed_buffer_mtx: Arc::new(Mutex::new(exposed_buffer)),
            working_buffer: Some(working_buffer),
            lower_to_keep: 0,
            sample_gauge: None,
            sample_window: None,
            max_sample_window: 0,
            can_handle_eos: true,
            has_new_position: false,
        }
    }

    // Get a reference on the exposed buffer mutex.
    pub fn get_exposed_buffer_mtx(&self) -> Arc<Mutex<Box<dyn SampleExtractor>>> {
        Arc::clone(&self.exposed_buffer_mtx)
    }

    pub fn cleanup(&mut self) {
        self.reset();
        self.audio_buffer.cleanup();

        {
            let exposed_buffer = &mut self.exposed_buffer_mtx.lock().unwrap();
            exposed_buffer.cleanup();
        }

        self.working_buffer.as_mut().unwrap().cleanup();
    }

    fn reset(&mut self) {
        self.state = gst::State::Null;
        self.samples_since_last_extract = 0;
        self.lower_to_keep = 0;
        self.sample_gauge = None;
        self.sample_window = None;
        self.max_sample_window = 0;
        self.can_handle_eos = true;
        self.has_new_position = false;
    }

    pub fn set_state(&mut self, state: gst::State) {
        self.state = state;
        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock().unwrap();
            exposed_buffer_box.set_state(state);
        }
        self.working_buffer.as_mut().unwrap().set_state(state);
    }

    pub fn clean_samples(&mut self) {
        self.audio_buffer.clean_samples();
        // Also reset basetime
        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock().unwrap();
            exposed_buffer_box.reset_basetime();
        }
        self.working_buffer.as_mut().unwrap().reset_basetime();
    }

    pub fn set_caps(&mut self, caps: &gst::CapsRef) {
        info!("Changing caps");
        let audio_info = gst_audio::AudioInfo::from_caps(caps).unwrap();

        self.reset();

        let rate = u64::from(audio_info.rate());
        let channels = audio_info.channels() as usize;

        let sample_duration = 1_000_000_000 / rate;
        self.max_sample_window = (QUEUE_SIZE_NS / sample_duration) as usize;
        let duration_for_1000_samples = 1_000_000_000_000f64 / (rate as f64);

        let mut channels: SmallVec<[AudioChannel; INLINE_CHANNELS]> =
            SmallVec::with_capacity(channels);
        if let Some(positions) = audio_info.positions() {
            for position in positions {
                channels.push(AudioChannel::new(*position));
            }
        };

        self.audio_buffer.init(audio_info);

        {
            let exposed_buffer = &mut self.exposed_buffer_mtx.lock().unwrap();
            exposed_buffer.set_sample_duration(sample_duration, duration_for_1000_samples);
            exposed_buffer.set_channels(&channels);
            exposed_buffer.new_segment();
        }

        let working_buffer = self.working_buffer.as_mut().unwrap();
        working_buffer.set_sample_duration(sample_duration, duration_for_1000_samples);
        working_buffer.set_channels(&channels);
        working_buffer.new_segment();
    }

    pub fn set_ref(&mut self, audio_ref: &gst::Element) {
        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock().unwrap();
            exposed_buffer_box.set_time_ref(audio_ref);
        }
        self.working_buffer
            .as_mut()
            .unwrap()
            .set_time_ref(audio_ref);
    }

    pub fn ignore_eos(&mut self) {
        self.can_handle_eos = false;
    }

    pub fn accept_eos(&mut self) {
        self.can_handle_eos = true;
    }

    pub fn handle_eos(&mut self) {
        if self.can_handle_eos {
            self.audio_buffer.handle_eos();
            // extract last samples and swap
            self.extract_samples();
            // do it again to update second extractor too
            // this is required in case of a subsequent seek
            // in the extractors' range
            self.extract_samples();
        }
        self.sample_gauge = None
    }

    pub fn have_gst_segment(&mut self, segment: &gst::Segment) {
        let segment_start = segment.get_start().get_value() as usize;
        self.audio_buffer.have_gst_segment(segment_start);
        self.sample_gauge = Some(0);
        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock().unwrap();
            exposed_buffer_box.new_segment();
        }
        self.working_buffer.as_mut().unwrap().new_segment();
    }

    pub fn push_gst_buffer(&mut self, buffer: &gst::Buffer) -> bool {
        // store incoming samples
        let sample_nb = self
            .audio_buffer
            .push_gst_buffer(buffer, self.lower_to_keep);
        self.samples_since_last_extract += sample_nb;

        let must_notify = if self.state != gst::State::Playing {
            if let Some(mut gauge) = self.sample_gauge.take() {
                gauge += sample_nb;
                let must_notify = self
                    .sample_window
                    .map_or(false, |sample_window| gauge >= sample_window);

                if !must_notify {
                    self.sample_gauge = Some(gauge);
                }
                // after the threshold is reached, the gauge is removed
                must_notify
            } else {
                false
            }
        } else {
            false
        };

        if must_notify || self.samples_since_last_extract >= EXTRACTION_THRESHOLD {
            // extract new samples and swap
            self.extract_samples();
            self.samples_since_last_extract = 0;

            if must_notify {
                self.sample_gauge = None;
            }
        }

        must_notify
    }

    // Update the working extractor with new samples and swap.
    fn extract_samples(&mut self) {
        let mut working_buffer = self.working_buffer.take().unwrap();
        working_buffer.extract_samples(&self.audio_buffer);

        // swap buffers
        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock().unwrap();
            // get latest state from the previously exposed buffer
            // in order to smoothen rendering between frames
            working_buffer.update_concrete_state(exposed_buffer_box.as_mut());
            mem::swap(exposed_buffer_box, &mut working_buffer);
        }

        self.lower_to_keep = working_buffer.get_lower();
        self.sample_window = working_buffer
            .get_requested_sample_window()
            .map(|req_sample_window| req_sample_window.min(self.max_sample_window));

        self.working_buffer = Some(working_buffer);
        // self.working_buffer is now the buffer previously in
        // self.exposed_buffer_mtx
    }

    pub fn refresh(&mut self) {
        // refresh with current conditions
        let mut working_buffer = self.working_buffer.take().unwrap();
        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock().unwrap();

            if self.state != gst::State::Playing {
                exposed_buffer_box.switch_to_paused();
            }

            // get latest state from the previously exposed buffer
            working_buffer.update_concrete_state(exposed_buffer_box.as_mut());

            // refresh working buffer
            working_buffer.refresh(&self.audio_buffer);

            // swap buffers
            mem::swap(exposed_buffer_box, &mut working_buffer);
        }

        self.lower_to_keep = working_buffer.get_lower();

        self.working_buffer = Some(working_buffer);
        // self.working_buffer is now the buffer previously in
        // self.exposed_buffer_mtx
    }
}
