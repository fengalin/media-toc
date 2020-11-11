use log::{debug, info};

use std::{
    mem,
    sync::{Arc, Mutex},
};

use metadata::Duration;

use super::{
    AudioBuffer, AudioChannel, SampleExtractor, SampleIndex, SampleIndexRange, INLINE_CHANNELS,
    QUEUE_SIZE,
};

const EXTRACTION_THRESHOLD: SampleIndexRange = SampleIndexRange::new(4096);

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
pub struct DoubleAudioBuffer<SE: SampleExtractor + 'static> {
    state: gst::State,
    audio_buffer: AudioBuffer,
    samples_since_last_extract: SampleIndex,
    exposed_buffer_mtx: Arc<Mutex<Box<SE>>>,
    working_buffer: Option<Box<SE>>,
    lower_to_keep: SampleIndex,
    sample_gauge: Option<SampleIndex>,
    sample_window: Option<SampleIndexRange>,
    max_sample_window: SampleIndexRange,
    can_handle_eos: bool, // accept / ignore eos (required for range seeks handling)
    has_new_position: bool,
}

impl<SE: SampleExtractor + 'static> DoubleAudioBuffer<SE> {
    // need 2 arguments for new as we can't clone buffers as they are known
    // as trait SampleExtractor
    pub fn new(
        buffer_duration: Duration,
        exposed_buffer: Box<SE>,
        working_buffer: Box<SE>,
    ) -> DoubleAudioBuffer<SE> {
        DoubleAudioBuffer {
            state: gst::State::Null,
            audio_buffer: AudioBuffer::new(buffer_duration),
            samples_since_last_extract: SampleIndex::default(),
            exposed_buffer_mtx: Arc::new(Mutex::new(exposed_buffer)),
            working_buffer: Some(working_buffer),
            lower_to_keep: SampleIndex::default(),
            sample_gauge: None,
            sample_window: None,
            max_sample_window: SampleIndexRange::default(),
            can_handle_eos: true,
            has_new_position: false,
        }
    }

    // Get a reference on the exposed buffer mutex.
    pub fn exposed_buffer_mtx(&self) -> Arc<Mutex<Box<SE>>> {
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
        self.samples_since_last_extract = SampleIndex::default();
        self.lower_to_keep = SampleIndex::default();
        self.sample_gauge = None;
        self.sample_window = None;
        self.max_sample_window = SampleIndexRange::default();
        self.can_handle_eos = true;
        self.has_new_position = false;
    }

    pub fn set_state(&mut self, state: gst::State) {
        debug!("changing state from {:?} to {:?}", self.state, state);
        self.state = state;
        self.working_buffer.as_mut().unwrap().set_gst_state(state);
        if state == gst::State::Paused {
            self.refresh();
        }
    }

    pub fn clean_samples(&mut self) {
        self.audio_buffer.clean_samples();
    }

    pub fn set_caps(&mut self, caps: &gst::CapsRef) {
        info!("changing caps");
        let audio_info = gst_audio::AudioInfo::from_caps(caps).unwrap();

        self.reset();

        let rate = u64::from(audio_info.rate());

        let sample_duration = Duration::from_frequency(rate);
        self.max_sample_window = SampleIndexRange::from_duration(QUEUE_SIZE, sample_duration);
        let duration_per_1000_samples = Duration::from_nanos(1_000_000_000_000u64 / rate);

        self.audio_buffer.init(&audio_info);

        let mut positions_opt = audio_info.positions().map(|positions| positions.iter());
        let channels = positions_opt
            .iter_mut()
            .flatten()
            .take(INLINE_CHANNELS)
            .map(|position| AudioChannel::new(*position));

        self.exposed_buffer_mtx
            .lock()
            .unwrap()
            .reset_sample_conditions();

        let working_buffer = self.working_buffer.as_mut().unwrap();
        working_buffer.set_sample_duration(sample_duration, duration_per_1000_samples);
        working_buffer.set_channels(channels);
    }

    pub fn set_ref(&mut self, audio_ref: &gst::Element) {
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
            self.refresh();
            // do it again to update second extractor too
            // this is required in case of a subsequent seek
            // in the extractors' range
            self.refresh();
        }
        self.sample_gauge = None
    }

    pub fn have_gst_segment(&mut self, segment: &gst::Segment) {
        self.audio_buffer
            .have_gst_segment(segment.get_start().get_value().into());
        self.sample_gauge = Some(SampleIndex::default());
    }

    pub fn push_gst_buffer(&mut self, buffer: &gst::Buffer) -> bool {
        // store incoming samples
        let sample_nb = self
            .audio_buffer
            .push_gst_buffer(buffer, self.lower_to_keep);
        self.samples_since_last_extract += sample_nb;

        let mut must_notify = false;
        if self.state != gst::State::Playing {
            if let Some(gauge) = self.sample_gauge.as_mut() {
                *gauge += sample_nb;
                let gauge = *gauge; // let go the ref on self.sample_gauge
                must_notify = self
                    .sample_window
                    .map_or(false, |sample_window| gauge >= sample_window);

                if must_notify {
                    self.sample_gauge = None;
                }
            }
        }

        if must_notify || self.samples_since_last_extract >= EXTRACTION_THRESHOLD {
            // extract new samples and swap
            self.refresh();
            self.samples_since_last_extract = SampleIndex::default();
        }

        must_notify
    }

    /// Refreshes the working extractor with new samples and swap.
    pub fn refresh(&mut self) {
        let mut working_buffer = self.working_buffer.take().unwrap();
        match working_buffer.extract_samples(&self.audio_buffer) {
            Some(status) => {
                self.lower_to_keep = status.lower;
                self.sample_window = Some(status.req_sample_window.min(self.max_sample_window));
            }
            None => self.sample_window = None,
        }

        // swap buffers
        {
            let exposed_buffer_box = &mut *self.exposed_buffer_mtx.lock().unwrap();
            mem::swap(exposed_buffer_box, &mut working_buffer);
        }

        self.working_buffer = Some(working_buffer);
        // self.working_buffer is now the buffer previously in
        // self.exposed_buffer_mtx
    }
}
