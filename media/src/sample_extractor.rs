use gstreamer as gst;
use gstreamer::ElementExtManual;

use metadata::Duration;

use super::{AudioBuffer, AudioChannel, SampleIndex, SampleIndexRange, Timestamp};

pub struct SampleExtractionState {
    pub sample_duration: Duration,
    pub duration_per_1000_samples: Duration,
    pub state: gst::State,
    audio_ref: Option<gst::Element>,
}

impl Default for SampleExtractionState {
    fn default() -> Self {
        SampleExtractionState {
            sample_duration: Duration::default(),
            duration_per_1000_samples: Duration::default(),
            state: gst::State::Null,
            audio_ref: None,
        }
    }
}

impl SampleExtractionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        *self = Self::default();
    }
}

pub trait SampleExtractor: Send {
    fn extraction_state(&self) -> &SampleExtractionState;
    fn extraction_state_mut(&mut self) -> &mut SampleExtractionState;

    fn cleanup(&mut self);

    fn set_state(&mut self, new_state: gst::State) {
        let state = self.extraction_state_mut();
        state.state = new_state;
    }

    fn set_time_ref(&mut self, audio_ref: &gst::Element) {
        self.extraction_state_mut().audio_ref = Some(audio_ref.clone());
    }

    fn set_channels(&mut self, channels: &[AudioChannel]);

    fn set_sample_duration(&mut self, per_sample: Duration, per_1000_samples: Duration);

    fn lower(&self) -> SampleIndex;

    fn req_sample_window(&self) -> Option<SampleIndexRange>;

    fn switch_to_paused(&mut self);

    // update self with concrete state of other
    // which is expected to be the same concrete type
    // this update is intended at smoothening the specific
    // extraction process by keeping conditions between frames
    fn update_concrete_state(&mut self, other: &mut Self);

    fn current_sample(&mut self) -> Option<(Timestamp, SampleIndex)> {
        let state = &self.extraction_state();

        let mut query = gst::query::Position::new(gst::Format::Time);
        if !state.audio_ref.as_ref().unwrap().query(&mut query) {
            return None;
        }

        let ts = Timestamp::new(query.get_result().get_value() as u64);
        Some((ts, ts.sample_index(state.sample_duration)))
    }

    // Update the extractions taking account new
    // samples added to the buffer and possibly a
    // different timestamps
    fn extract_samples(&mut self, audio_buffer: &AudioBuffer);

    // Refresh the extractionm in its current sample range
    // and timestamps
    fn refresh(&mut self, audio_buffer: &AudioBuffer);
}
