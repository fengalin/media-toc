use gst::prelude::*;

use metadata::Duration;
use std::sync::RwLock;

use crate::{AudioBuffer, AudioChannel, SampleIndex, SampleIndexRange, Timestamp};

#[derive(Debug, Default)]
pub struct State {
    audio_ref: Option<gst::Element>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cleanup(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug)]
pub struct RenderingStatus {
    pub lower: SampleIndex,
    pub req_sample_window: SampleIndexRange,
}

pub trait Renderer: Send {
    fn cleanup(&mut self);
    fn state(&self) -> &RwLock<State>;

    fn set_time_ref(&mut self, audio_ref: &gst::Element) {
        self.state().write().unwrap().audio_ref = Some(audio_ref.clone());
    }

    fn set_sample_cndt(
        &mut self,
        _per_sample: Duration,
        _per_1000_samples: Duration,
        _channels: &mut dyn Iterator<Item = AudioChannel>,
    );

    fn reset_sample_cndt(&mut self);

    fn first_visible_sample(&self) -> Option<SampleIndex>;

    fn current_ts(&self) -> Option<Timestamp> {
        let mut query = gst::query::Position::new(gst::Format::Time);
        if !self
            .state()
            .read()
            .unwrap()
            .audio_ref
            .as_ref()?
            .query(&mut query)
        {
            return None;
        }

        Some(Timestamp::new(query.result().value() as u64))
    }

    // FIXME remove print_state
    fn print_state(&self);

    fn freeze(&mut self);
    fn release(&mut self);
    fn seek_start(&mut self);
    fn seek_done(&mut self, ts: Timestamp);
    fn cancel_seek(&mut self);
    fn render(&mut self, audio_buffer: &AudioBuffer) -> Option<RenderingStatus>;
}
