use glib::subclass::prelude::*;
use glib::GBoxed;

use log::info;

use std::sync::{Arc, Mutex};

use super::Renderer;
use crate::{AudioBuffer, AudioChannel, SampleIndex, SampleIndexRange, Timestamp, INLINE_CHANNELS};
use metadata::Duration;

const EXTRACTION_THRESHOLD: SampleIndexRange = SampleIndexRange::new(4096);

pub trait DoubleRendererImpl: std::fmt::Debug + Send + 'static {
    fn swap(&mut self);
    fn working(&mut self) -> &mut dyn Renderer;
    fn cleanup(&mut self);
    fn set_sample_cndt(
        &mut self,
        _per_sample: Duration,
        _per_1000_samples: Duration,
        _channels: &mut dyn Iterator<Item = AudioChannel>,
    );
}

#[derive(Clone, Debug, GBoxed)]
#[gboxed(type_name = "DoubleRendererImpl")]
pub struct GBoxedDoubleRendererImpl(Arc<Mutex<Option<Box<dyn DoubleRendererImpl>>>>);

impl From<&GBoxedDoubleRendererImpl> for Option<Box<dyn DoubleRendererImpl>> {
    fn from(gboxed_: &GBoxedDoubleRendererImpl) -> Self {
        gboxed_.0.lock().unwrap().take()
    }
}

impl From<Box<dyn DoubleRendererImpl>> for GBoxedDoubleRendererImpl {
    fn from(dbl_visu_renderer_impl: Box<dyn DoubleRendererImpl>) -> Self {
        GBoxedDoubleRendererImpl(Arc::new(Mutex::new(Some(dbl_visu_renderer_impl))))
    }
}

/// A double buffering mechanism to render visualizations of the audio signal.
///
/// The `DoubleRenderer` is reponsible for ensuring a thread safe double buffering
/// mechanism that receives buffers from GStreamer, prepares an extraction of
/// these samples and presents the most recent extraction to an external
/// mechanism (e.g. UI).
/// The `DoubleRenderer` delegates the actual rendering to a `DoubleRendererImpl`.
#[derive(Debug)]
pub struct DoubleRenderer {
    impl_: Box<dyn DoubleRendererImpl>,
    state: gst::State,
    audio_buffer: AudioBuffer,
    samples_since_last_extract: SampleIndex,
    lower_to_keep: SampleIndex,
    sample_gauge: Option<SampleIndex>,
    sample_window: Option<SampleIndexRange>,
    max_sample_window: SampleIndexRange,
    sample_duration: Duration,
    has_new_position: bool,
}

impl DoubleRenderer {
    // need 2 arguments for new as we can't clone buffers as they are known
    // as trait SampleExtractor
    pub fn new(
        mut impl_: Box<dyn DoubleRendererImpl>,
        buffer_duration: Duration,
        clock_ref: &gst::Element,
    ) -> Self {
        impl_.working().set_time_ref(clock_ref);

        DoubleRenderer {
            impl_,
            state: gst::State::Null,
            audio_buffer: AudioBuffer::new(buffer_duration),
            samples_since_last_extract: SampleIndex::default(),
            lower_to_keep: SampleIndex::default(),
            sample_gauge: None,
            sample_window: None,
            max_sample_window: SampleIndexRange::default(),
            sample_duration: Duration::default(),
            has_new_position: false,
        }
    }

    pub fn into_impl(self) -> Box<dyn DoubleRendererImpl> {
        self.impl_
    }

    pub fn cleanup(&mut self) {
        self.reset();
        self.audio_buffer.cleanup();
        self.impl_.cleanup();
    }

    fn reset(&mut self) {
        self.state = gst::State::Null;
        self.samples_since_last_extract = SampleIndex::default();
        self.lower_to_keep = SampleIndex::default();
        self.sample_gauge = None;
        self.sample_window = None;
        self.max_sample_window = SampleIndexRange::default();
        self.sample_duration = Duration::default();
        self.has_new_position = false;
    }

    pub fn set_caps(&mut self, caps: &gst::CapsRef) {
        info!("changing caps");
        let audio_info = gst_audio::AudioInfo::from_caps(caps).unwrap();

        self.reset();

        let rate = u64::from(audio_info.rate());

        self.sample_duration = Duration::from_frequency(rate);
        self.max_sample_window = SampleIndexRange::from_duration(
            self.audio_buffer.buffer_duration,
            self.sample_duration,
        );
        let duration_per_1000_samples = Duration::from_nanos(1_000_000_000_000u64 / rate);

        self.audio_buffer.init(&audio_info);

        let mut positions_opt = audio_info.positions().map(|positions| positions.iter());
        let mut channels = positions_opt
            .iter_mut()
            .flatten()
            .take(INLINE_CHANNELS)
            .map(|position| AudioChannel::new(*position));

        self.impl_.set_sample_cndt(
            self.sample_duration,
            duration_per_1000_samples,
            &mut channels,
        );
    }

    pub fn handle_eos(&mut self) {
        self.audio_buffer.handle_eos();
        // extract last samples and swap
        self.refresh();
        // do it again to update second extractor too
        // this is required in case of a subsequent seek
        // in the extractors' range
        self.refresh();
        self.sample_gauge = None;
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

        let must_notify = if let Some(gauge) = self.sample_gauge.as_mut() {
            *gauge += sample_nb;
            let gauge = *gauge; // let go the ref on self.sample_gauge
            let must_notify = self
                .sample_window
                .map_or(false, |sample_window| gauge >= sample_window);

            if must_notify {
                self.sample_gauge = None;
            }

            must_notify
        } else {
            false
        };

        if must_notify || self.samples_since_last_extract >= EXTRACTION_THRESHOLD {
            // extract new samples and swap
            self.refresh();
            self.samples_since_last_extract = SampleIndex::default();
        }

        must_notify
    }

    pub fn seek_start(&mut self) {
        self.impl_.working().seek_start();
    }

    pub fn seek_done(&mut self, ts: Timestamp) {
        self.impl_.working().seek_done(ts);
    }

    pub fn cancel_seek(&mut self) {
        self.impl_.working().cancel_seek();
    }

    /// Refreshes the working extractor with new samples and swap.
    pub fn refresh(&mut self) {
        match self.impl_.working().render(&self.audio_buffer) {
            Some(status) => {
                self.lower_to_keep = status.lower;
                self.sample_window = Some(status.req_sample_window.min(self.max_sample_window));
            }
            None => self.sample_window = None,
        }

        self.impl_.swap();
    }

    /// Finds the first timestamp for a 2 steps seek in Paused state.
    ///
    /// This is used as an attempt to center the waveform on the target timestamp.
    pub fn first_ts_for_two_steps_seek(&self, target: impl Into<Timestamp>) -> Option<Timestamp> {
        self.sample_window.and_then(|sample_window| {
            let target_index = target.into().sample_index(self.sample_duration);
            let wanted_first_index = target_index.saturating_sub_range(sample_window / 2);

            if self.audio_buffer.lower <= wanted_first_index
                && wanted_first_index < self.audio_buffer.upper
            {
                // Wanted first index already in current buffer range
                if target_index < self.audio_buffer.upper {
                    // ... and so is target => don't need to perform a two steps seek
                    None
                } else {
                    Some(
                        self.audio_buffer
                            .upper
                            .min(target_index)
                            .as_ts(self.sample_duration),
                    )
                }
            } else {
                // wanted_first_index not in range
                Some(wanted_first_index.as_ts(self.sample_duration))
            }
        })
    }
}
