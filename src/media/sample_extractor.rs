use gstreamer as gst;
use gstreamer::ElementExtManual;

use std::any::Any;

use super::{AudioBuffer, AudioChannel, SampleIndex, Timestamp};

pub struct SampleExtractionState {
    pub sample_duration: u64,
    pub duration_per_1000_samples: f64,
    pub state: gst::State,
    audio_ref: Option<gst::Element>,
    pub basetime: Option<(u64, u64)>, // (base_time, base_frame_time)
    pub last_ts: Timestamp,
    pub is_stable: bool, // post-"new segment" position stabilization flag
}

impl SampleExtractionState {
    pub fn new() -> Self {
        SampleExtractionState {
            sample_duration: 0,
            duration_per_1000_samples: 0f64,
            state: gst::State::Null,
            audio_ref: None,
            basetime: None,
            last_ts: Timestamp::default(),
            is_stable: false,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        self.sample_duration = 0;
        self.duration_per_1000_samples = 0f64;
        self.state = gst::State::Null;
        self.audio_ref = None;
        self.basetime = None;
        self.last_ts = Timestamp::default();
        self.is_stable = false;
    }

    pub fn reset_basetime(&mut self) {
        self.is_stable = false;
        self.basetime = None;
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
        state.reset_basetime();
    }

    fn set_time_ref(&mut self, audio_ref: &gst::Element) {
        self.get_extraction_state_mut().audio_ref = Some(audio_ref.clone());
    }

    #[inline]
    fn reset_basetime(&mut self) {
        self.get_extraction_state_mut().reset_basetime();
    }

    fn new_segment(&mut self) {
        self.reset_basetime();
        self.seek_complete();
    }

    fn seek_complete(&mut self);

    fn set_channels(&mut self, channels: &[AudioChannel]);

    fn set_sample_duration(&mut self, per_sample: u64, per_1000_samples: f64);

    fn get_lower(&self) -> SampleIndex;

    fn get_requested_sample_window(&self) -> Option<SampleIndex>;

    fn switch_to_paused(&mut self);

    // update self with concrete state of other
    // which is expected to be the same concrete type
    // this update is intended at smoothening the specific
    // extraction process by keeping conditions between frames
    fn update_concrete_state(&mut self, other: &mut dyn SampleExtractor);

    fn get_current_sample(
        &mut self,
        last_frame_time: u64,
        next_frame_time: u64,
    ) -> (Timestamp, SampleIndex) {
        let state = &mut self.get_extraction_state_mut();

        let ts = match state.state {
            gst::State::Playing => {
                let computed_position =
                    state.basetime.as_ref().map(|(base_time, base_frame_time)| {
                        Timestamp::new((next_frame_time - base_frame_time) * 1_000 + base_time)
                    });

                if state.is_stable {
                    computed_position.expect("get_current_sample is_stable but no basetime")
                } else {
                    let mut query = gst::Query::new_position(gst::Format::Time);
                    if state.audio_ref.as_ref().unwrap().query(&mut query) {
                        // approximate current position as being exactly between last frame
                        // and next frame
                        let base_time = query.get_result().get_value() as u64;
                        let half_interval = (next_frame_time - last_frame_time) * 1_000 / 2;
                        let position = base_time + half_interval;

                        if let Some(computed_position) = computed_position {
                            let delta = position as i64 - computed_position.as_i64();
                            if (delta.abs() as u64) < half_interval {
                                // computed position is now close enough to the actual position
                                state.is_stable = true;
                            }
                        }

                        state.basetime = Some((base_time, (next_frame_time + last_frame_time) / 2));

                        Timestamp::new(position)
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
    // different position
    fn extract_samples(&mut self, audio_buffer: &AudioBuffer);

    // Refresh the extractionm in its current sample range
    // and position.
    fn refresh(&mut self, audio_buffer: &AudioBuffer);
}
