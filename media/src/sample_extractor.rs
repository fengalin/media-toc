use gstreamer as gst;
use gstreamer::ElementExtManual;

use metadata::Duration;

use super::{AudioBuffer, AudioChannel, SampleIndex, SampleIndexRange, Timestamp};

pub struct State {
    pub gst_state: gst::State,
    audio_ref: Option<gst::Element>,
}

impl Default for State {
    fn default() -> Self {
        State {
            gst_state: gst::State::Null,
            audio_ref: None,
        }
    }
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        *self = Self::default();
    }
}

pub trait SampleExtractor: Send {
    fn with_state<Out>(&self, f: impl FnOnce(&State) -> Out) -> Out;
    fn with_state_mut<Out>(&mut self, f: impl FnOnce(&mut State) -> Out) -> Out;

    fn cleanup(&mut self);

    fn set_gst_state(&mut self, gst_state: gst::State) {
        self.with_state_mut(|state| state.gst_state = gst_state);
    }

    fn set_time_ref(&mut self, audio_ref: &gst::Element) {
        self.with_state_mut(|state| state.audio_ref = Some(audio_ref.clone()))
    }

    fn set_channels(&mut self, channels: impl Iterator<Item = AudioChannel>);

    fn set_sample_duration(&mut self, per_sample: Duration, per_1000_samples: Duration);

    fn reset_sample_conditions(&mut self);

    fn lower(&self) -> SampleIndex;

    fn req_sample_window(&self) -> Option<SampleIndexRange>;

    fn current_ts(&self) -> Option<Timestamp> {
        self.with_state(|state| {
            let mut query = gst::query::Position::new(gst::Format::Time);
            if !state.audio_ref.as_ref()?.query(&mut query) {
                return None;
            }

            Some(Timestamp::new(query.get_result().get_value() as u64))
        })
    }

    // Update the extractions taking account new
    // samples added to the buffer and possibly a
    // different timestamps
    fn extract_samples(&mut self, audio_buffer: &AudioBuffer);
}
