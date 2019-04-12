use gstreamer as gst;
use gstreamer::ElementExtManual;

use std::any::Any;

use super::{AudioBuffer, AudioChannel, Duration, SampleIndex, SampleIndexRange, Timestamp};

pub struct SampleExtractionState {
    pub sample_duration: Duration,
    pub duration_per_1000_samples: Duration,
    pub state: gst::State,
    audio_ref: Option<gst::Element>,
    pub base_ts: Option<(Timestamp, Timestamp)>, // (base_ts, base_frame_ts)
    pub last_ts: Timestamp,
    pub is_stable: bool, // post-"new segment" position stabilization flag
}

impl SampleExtractionState {
    pub fn new() -> Self {
        SampleExtractionState {
            sample_duration: Duration::default(),
            duration_per_1000_samples: Duration::default(),
            state: gst::State::Null,
            audio_ref: None,
            base_ts: None,
            last_ts: Timestamp::default(),
            is_stable: false,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        self.sample_duration = Duration::default();
        self.duration_per_1000_samples = Duration::default();
        self.state = gst::State::Null;
        self.audio_ref = None;
        self.base_ts = None;
        self.last_ts = Timestamp::default();
        self.is_stable = false;
    }

    pub fn reset_base_ts(&mut self) {
        self.is_stable = false;
        self.base_ts = None;
    }
}

pub trait SampleExtractor: Send {
    fn as_mut_any(&mut self) -> &mut dyn Any;
    fn as_any(&self) -> &dyn Any;
    fn get_extraction_state(&self) -> &SampleExtractionState;
    fn get_extraction_state_mut(&mut self) -> &mut SampleExtractionState;

    fn cleanup(&mut self);

    fn set_state(&mut self, new_state: gst::State) {
        let state = self.get_extraction_state_mut();
        state.state = new_state;
        state.reset_base_ts();
    }

    fn set_time_ref(&mut self, audio_ref: &gst::Element) {
        self.get_extraction_state_mut().audio_ref = Some(audio_ref.clone());
    }

    #[inline]
    fn reset_base_ts(&mut self) {
        self.get_extraction_state_mut().reset_base_ts();
    }

    fn new_segment(&mut self) {
        self.reset_base_ts();
        self.seek_complete();
    }

    fn seek_complete(&mut self);

    fn set_channels(&mut self, channels: &[AudioChannel]);

    fn set_sample_duration(&mut self, per_sample: Duration, per_1000_samples: Duration);

    fn get_lower(&self) -> SampleIndex;

    fn get_requested_sample_window(&self) -> Option<SampleIndexRange>;

    fn switch_to_paused(&mut self);

    // update self with concrete state of other
    // which is expected to be the same concrete type
    // this update is intended at smoothening the specific
    // extraction process by keeping conditions between frames
    fn update_concrete_state(&mut self, other: &mut dyn SampleExtractor);

    fn get_current_sample(
        &mut self,
        last_frame_ts: Timestamp,
        next_frame_ts: Timestamp,
    ) -> (Timestamp, SampleIndex) {
        let state = &mut self.get_extraction_state_mut();

        let ts = match state.state {
            gst::State::Playing => {
                let computed_ts = state
                    .base_ts
                    .clone()
                    .map(|(base_ts, base_frame_ts)| base_ts + (next_frame_ts - base_frame_ts));

                if state.is_stable {
                    computed_ts.expect("get_current_sample is_stable but no base_ts")
                } else {
                    let mut query = gst::Query::new_position(gst::Format::Time);
                    if state.audio_ref.as_ref().unwrap().query(&mut query) {
                        // approximate current timestamp as being exactly between last frame
                        // and next frame
                        let base_ts = Timestamp::new(query.get_result().get_value() as u64);
                        let half_interval = (next_frame_ts - last_frame_ts) / 2;
                        let ts = base_ts + half_interval;

                        if let Some(computed_ts) = computed_ts {
                            let delta = ts.as_i64() - computed_ts.as_i64();
                            if Duration::from_nanos(delta.abs() as u64) < half_interval {
                                // computed timestamp is now close enough to the actual timestamp
                                state.is_stable = true;
                            }
                        }

                        state.base_ts =
                            Some((base_ts, last_frame_ts.get_halfway_to(next_frame_ts)));

                        ts
                    } else {
                        state.last_ts
                    }
                }
            }
            gst::State::Paused => {
                let mut query = gst::Query::new_position(gst::Format::Time);
                if state.audio_ref.as_ref().unwrap().query(&mut query) {
                    Timestamp::new(query.get_result().get_value() as u64)
                } else {
                    state.last_ts
                }
            }
            _ => state.last_ts,
        };

        state.last_ts = ts;

        (ts, ts.get_sample_index(state.sample_duration))
    }

    // Update the extractions taking account new
    // samples added to the buffer and possibly a
    // different timestamps
    fn extract_samples(&mut self, audio_buffer: &AudioBuffer);

    // Refresh the extractionm in its current sample range
    // and timestamps
    fn refresh(&mut self, audio_buffer: &AudioBuffer);
}
