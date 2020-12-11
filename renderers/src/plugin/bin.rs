use glib::{clone, glib_object_subclass, subclass::prelude::*};
use gst::{
    gst_debug, gst_error, gst_info, gst_trace, gst_warning, prelude::*, subclass::prelude::*,
    ClockTime, GhostPad, Seqnum,
};

use lazy_static::lazy_static;

use std::sync::{Arc, Mutex, MutexGuard};

use crate::{
    generic::{GBoxedDoubleRendererImpl, WindowTimestamps},
    plugin,
};

pub const NAME: &str = "mediatocrendererbin";

lazy_static! {
    static ref CAT: gst::DebugCategory = gst::DebugCategory::new(
        NAME,
        gst::DebugColorFlags::empty(),
        Some("media-toc Renderer Bin"),
    );
}

static PROPERTIES: &[glib::subclass::Property; 3] = &plugin::renderer::PROPERTIES;

/// Seek state machine state for two stages seeks.
///
/// A two stages seek allows centering the audio representation around
/// target position in Paused mode. This helps the user visualize
/// the context around current position and interact with the
/// application.
///
/// A two stages seek is initiated when the [`Pipeline`](gst::Pipeline)
/// is in Paused mode and the position in the stream is appropriate.
/// The sequence unfolds as follows:
///
/// - A user seek is received and the conditions are appropriate for
///   a two stages seek.
/// - When the [`Segment`](gst::Segment) from the user seek is received,
///   a new seek is immediately sent upstream. This first stage seek
///   aims at retrieving the samples required to render the audio
///   visualisation preceeding the user target position.
/// - Once the renderer has received enough samples, a second seek
///   is emitted in order to set the [`Pipeline`](gst::Pipeline) back to
///   the user requested position.
///
/// When the [`Renderer`] [`Element`](gst::Element) is used via a
/// [`RendererBin`], the first stage seek can be filtered so that only
/// the rendering elements are affected. This allows reducing resources
/// usage.

/// Field indicating the [`RendererBin`] that this needs to be filtered.
///
/// When the [`RendererBin`] receives a seek [`Event`](gst::Event) containing
/// this field, it must send the events and buffers in the [`Segment`](gst::Segment)
/// with the same [`Seqnum`](Seqnum) only to the rendering elements.
#[derive(Clone, Copy, Debug)]
enum SeekState {
    None,
    InitSegment {
        expected_seqnum: Seqnum,
        remaining_streams: usize,
        stage_1_start: ClockTime,
        stage_1_end: ClockTime,
        target_ts: ClockTime,
    },
    Stage1Segment {
        expected_seqnum: Seqnum,
        remaining_streams: usize,
        accepts_audio_buffers: bool,
        target_ts: ClockTime,
    },
    Stage2Segment {
        expected_seqnum: Seqnum,
        remaining_streams: usize,
    },
}

impl SeekState {
    fn expected_seqnum(&self) -> Option<Seqnum> {
        use SeekState::*;
        match self {
            InitSegment {
                expected_seqnum, ..
            }
            | Stage1Segment {
                expected_seqnum, ..
            }
            | Stage2Segment {
                expected_seqnum, ..
            } => Some(*expected_seqnum),
            _ => Option::None,
        }
    }
}

impl Default for SeekState {
    fn default() -> Self {
        SeekState::None
    }
}

#[derive(Clone, Copy, Debug)]
enum PlaybackState {
    Paused,
    Playing,
    Prepared,
    Stopped,
    Unprepared,
}

impl Default for PlaybackState {
    fn default() -> Self {
        PlaybackState::Unprepared
    }
}

impl PlaybackState {
    fn is_playing(&self) -> bool {
        matches!(self, PlaybackState::Playing)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct State {
    playback: PlaybackState,
    seek: SeekState,
    stream_count: usize,
}

pub struct RendererBin {
    state: Arc<Mutex<State>>,
    audio_sinkpad: GhostPad,
    audio_srcpad: GhostPad,
    renderer: gst::Element,
    renderer_queue_sinkpad: gst::Pad,
    renderer_queue: gst::Element,
    audio_tee: gst::Element,
    audio_queue: gst::Element,
    audio_queue_sinkpad: gst::Pad,
    video_sinkpad: GhostPad,
    video_srcpad: GhostPad,
    video_queue: gst::Element,
}

#[derive(Debug, PartialEq)]
enum PadStream {
    Audio,
    Video,
}

/// Sink Pads handlers.
impl RendererBin {
    fn sink_chain(
        &self,
        pad_stream: PadStream,
        pad: &GhostPad,
        bin: &plugin::RendererBin,
        buffer: gst::Buffer,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        use SeekState::*;

        let seek_state = self.state.lock().unwrap().seek;
        match seek_state {
            None | Stage2Segment { .. } => (),
            Stage1Segment {
                accepts_audio_buffers,
                ..
            } => {
                if accepts_audio_buffers && PadStream::Audio == pad_stream {
                    return self.renderer_queue_sinkpad.chain(buffer);
                }

                return Ok(gst::FlowSuccess::Ok);
            }
            InitSegment { .. } => return Ok(gst::FlowSuccess::Ok),
        }
        drop(seek_state);

        pad.chain_default(Some(bin), buffer)
    }

    fn sink_event(
        &self,
        pad_stream: PadStream,
        pad: &GhostPad,
        bin: &plugin::RendererBin,
        mut event: gst::Event,
    ) -> bool {
        use gst::EventView::*;
        use SeekState::*;

        match event.view() {
            // FIXME only retain actually selected streams among Video & Audio
            StreamCollection(evt) => {
                self.state.lock().unwrap().stream_count = evt.get_stream_collection().len();
            }
            FlushStart(_) | FlushStop(_) => {
                let state = self.state.lock().unwrap();
                if let Some(expected_seqnum) = state.seek.expected_seqnum() {
                    let seqnum = event.get_seqnum();
                    if expected_seqnum == seqnum {
                        match state.seek {
                            InitSegment { .. } => (),
                            Stage1Segment { .. } => {
                                let event_type = event.get_type();
                                if PadStream::Audio != pad_stream {
                                    gst_debug!(
                                        CAT,
                                        obj: pad,
                                        "filtering non-audio stage 1 {} {:?}",
                                        event_type,
                                        seqnum
                                    );
                                    return true;
                                }

                                gst_debug!(
                                    CAT,
                                    obj: pad,
                                    "forwarding stage 1 audio {} {:?}",
                                    event_type,
                                    seqnum
                                );

                                drop(state);
                                let ret = self.renderer_queue_sinkpad.send_event(event);
                                if !ret {
                                    gst_error!(
                                        CAT,
                                        obj: pad,
                                        "failed to forward stage 1 audio {} {:?}",
                                        event_type,
                                        seqnum
                                    );
                                    self.state.lock().unwrap().seek = SeekState::None;
                                }

                                return ret;
                            }
                            Stage2Segment { .. } => {
                                // Don't flush the renderer queue
                                if PadStream::Audio == pad_stream {
                                    return self.audio_queue_sinkpad.send_event(event);
                                }
                            }
                            None => unreachable!(),
                        }
                    } else {
                        gst_debug!(
                            CAT,
                            obj: pad,
                            "unexpected {} {:?}",
                            event.get_type(),
                            seqnum
                        );
                        // forward and wait for the Segment before deciding what to do
                        // FIXME might also cancel right away
                    }
                }
            }
            Segment(_) => {
                let mut state = self.state.lock().unwrap();
                if state.playback.is_playing() {
                    drop(state);
                    return pad.event_default(Some(bin), event);
                }

                let seqnum = event.get_seqnum();

                match &mut state.seek {
                    None => return self.handle_new_segment(state, pad_stream, pad, bin, event),
                    InitSegment {
                        expected_seqnum,
                        remaining_streams,
                        stage_1_start,
                        stage_1_end,
                        target_ts,
                    } => {
                        if seqnum != *expected_seqnum {
                            return self.unexpected_segment(state, pad_stream, pad, bin, event);
                        }

                        if PadStream::Audio == pad_stream {
                            event
                                .make_mut()
                                .structure_mut()
                                .set(plugin::SegmentField::InitTwoStages.as_str(), &"");
                        }

                        if !pad.event_default(Some(bin), event) {
                            gst_error!(
                                CAT,
                                obj: pad,
                                "failed to forward initial Segment {:?} downstream",
                                seqnum,
                            );
                            state.seek = None;

                            return false;
                        }

                        *remaining_streams -= 1;
                        if *remaining_streams > 0 {
                            return true;
                        }

                        let stage_1_start = *stage_1_start;
                        let stage_1_end = *stage_1_end;
                        let target_ts = *target_ts;
                        self.send_stage_1_seek(
                            state,
                            pad,
                            bin,
                            stage_1_start,
                            stage_1_end,
                            target_ts,
                        );

                        return true;
                    }
                    Stage1Segment {
                        expected_seqnum,
                        remaining_streams,
                        accepts_audio_buffers,
                        ..
                    } => {
                        if seqnum != *expected_seqnum {
                            return self.unexpected_segment(state, pad_stream, pad, bin, event);
                        }

                        *remaining_streams -= 1;

                        if PadStream::Audio != pad_stream {
                            gst_debug!(
                                CAT,
                                obj: pad,
                                "filtering stage 1 non-audio segment {:?}",
                                seqnum,
                            );
                            return true;
                        }

                        gst_debug!(
                            CAT,
                            obj: pad,
                            "forwarding stage 1 audio segment {:?}",
                            seqnum,
                        );

                        *accepts_audio_buffers = true;
                        event
                            .make_mut()
                            .structure_mut()
                            .set(plugin::SegmentField::Stage1.as_str(), &"");

                        drop(state);
                        if self.renderer_queue_sinkpad.send_event(event) {
                            return true;
                        } else {
                            gst_error!(
                                CAT,
                                obj: pad,
                                "failed to forward stage 1 audio segment {:?}",
                                seqnum,
                            );
                            self.state.lock().unwrap().seek = SeekState::None;
                            return false;
                        }
                    }
                    Stage2Segment {
                        expected_seqnum,
                        remaining_streams,
                    } => {
                        if seqnum != *expected_seqnum {
                            return self.unexpected_segment(state, pad_stream, pad, bin, event);
                        }

                        if PadStream::Audio == pad_stream {
                            gst_debug!(CAT, obj: pad, "got stage 2 audio segment {:?}", seqnum,);
                            event
                                .make_mut()
                                .structure_mut()
                                .set(plugin::SegmentField::Stage2.as_str(), &"");
                        } else {
                            gst_debug!(CAT, obj: pad, "got stage 2 non-audio segment {:?}", seqnum);
                        }

                        *remaining_streams -= 1;
                        if *remaining_streams == 0 {
                            gst_info!(CAT, obj: pad, "got all stage 2 segments {:?}", seqnum);

                            state.seek = None;
                        }
                    }
                }
            }
            SegmentDone(_) => {
                // FIXME can't send stage 2 seek as soon as SegmentDone is received
                // by bin since it takes time for the buffers to reach the renderer
                // meaning that buffers can be flushed before they reach the renderer
                // => so the renderer must be in charge of sending the stage 2 seek
                // upon reception of SegmentDone.
                // It's kind of unfortunate since it spreads the seek logic
                // among the bin and the renderer element.
                // Maybe we could use a custom upstream event that would indicate
                // that the renderer element got the SegmentDone.

                let seqnum = event.get_seqnum();

                let mut state = self.state.lock().unwrap();

                match &mut state.seek {
                    Stage1Segment {
                        expected_seqnum, ..
                    } => {
                        if seqnum != *expected_seqnum {
                            gst_debug!(CAT, obj: pad, "unexpected SegmentDone {:?}", seqnum);
                            state.seek = None;
                            return pad.event_default(Some(bin), event);
                        }

                        if PadStream::Audio == pad_stream {
                            gst_debug!(CAT, obj: pad, "forwarding stage 1 audio SegmentDone");
                            if !self.renderer_queue_sinkpad.send_event(event) {
                                gst_error!(CAT, obj: pad, "failed to forward SegmentDone");
                                return false;
                            }
                        } else {
                            gst_debug!(CAT, obj: pad, "filtering stage 1 non-audio SegmentDone");
                        };

                        return true;
                    }
                    other => {
                        todo!("SegmentDone in {:?}", other);
                    }
                }
            }
            _ => (),
        }

        pad.event_default(Some(bin), event)
    }
}

/// Seek handler.
impl RendererBin {
    fn unexpected_segment(
        &self,
        state: MutexGuard<State>,
        pad_stream: PadStream,
        pad: &GhostPad,
        bin: &plugin::RendererBin,
        event: gst::Event,
    ) -> bool {
        gst_warning!(
            CAT,
            obj: pad,
            "{:?}: got {:?} with {:?}",
            state.seek,
            event.get_type(),
            event.get_seqnum(),
        );

        self.handle_new_segment(state, pad_stream, pad, bin, event)
    }

    fn are_in_window(start: ClockTime, end: ClockTime, window_ts: &WindowTimestamps) -> bool {
        if window_ts.start.is_none() || window_ts.end.is_none() {
            return false;
        }

        let window_start = window_ts.start.unwrap();
        if start.is_none() || start < window_start {
            return false;
        }

        let window_end = window_ts.end.unwrap();
        if end.is_none()
            || start >= window_end
            // Take a margin with the requested end ts because it might be rounded up
            // FIXME find a better way to deal with this
            || end.saturating_sub(window_ts.window / 10).unwrap() > (window_start + window_end)
        {
            return false;
        }

        true
    }

    fn first_ts_for_two_stages_seek(
        target: ClockTime,
        window_ts: &WindowTimestamps,
    ) -> Option<ClockTime> {
        let wanted_first = ClockTime::from_nseconds(
            target
                .nanoseconds()?
                .checked_sub(window_ts.window.nanoseconds()? / 2)?,
        );
        if window_ts.start.is_some()
            && wanted_first >= window_ts.start.unwrap()
            && window_ts.end.is_some()
            && wanted_first < window_ts.end.unwrap()
        {
            return None;
        }

        Some(wanted_first)
    }

    fn handle_new_segment(
        &self,
        mut state: MutexGuard<State>,
        pad_stream: PadStream,
        pad: &GhostPad,
        bin: &plugin::RendererBin,
        mut event: gst::Event,
    ) -> bool {
        state.seek = SeekState::None;

        // FIXME only consider seek options if audio stream is active
        // otherwise just foward as is.

        let segment = match event.view() {
            gst::EventView::Segment(segment_evt) => segment_evt.get_segment(),
            other => unreachable!("unexpected {:?}", other),
        };

        let segment_seqnum = event.get_seqnum();

        let segment = match segment.downcast_ref::<gst::format::Time>() {
            Some(segment) => segment,
            None => {
                // Not a Time segment, keep the event as is
                drop(state);
                gst_debug!(CAT, obj: pad, "not Time {:?} {:?}", segment, segment_seqnum);
                return pad.event_default(Some(bin), event);
            }
        };

        let target_ts = segment.get_time();

        // FIXME handle playing backward

        let window_ts_res = self
            .renderer
            .emit(plugin::GET_WINDOW_TIMESTAMPS_SIGNAL, &[]);
        let window_ts = match window_ts_res.as_ref() {
            Ok(Some(window_ts)) => {
                window_ts
                    .get_some::<&WindowTimestamps>()
                    .unwrap_or_else(|err| {
                        panic!(
                            "signal {} returned value: {}",
                            plugin::GET_WINDOW_TIMESTAMPS_SIGNAL,
                            err
                        )
                    })
            }
            Ok(None) => {
                drop(state);
                gst_debug!(
                    CAT,
                    obj: pad,
                    "unknown rendering conditions for segment starting @ {}",
                    target_ts,
                );
                return pad.event_default(Some(bin), event);
            }
            Err(err) => {
                panic!(
                    "renderer bin failed to emit signal {}: {}",
                    plugin::GET_WINDOW_TIMESTAMPS_SIGNAL,
                    err,
                );
            }
        };

        let stop_ts = segment.get_stop();
        if Self::are_in_window(target_ts, stop_ts, window_ts) {
            drop(state);
            gst_debug!(
                CAT,
                obj: pad,
                "segment [{}, {}] in window {:?}",
                target_ts,
                stop_ts,
                segment_seqnum,
            );
            event
                .make_mut()
                .structure_mut()
                .set(plugin::SegmentField::InWindow.as_str(), &"");
            return pad.event_default(Some(bin), event);
        }

        if let Some(stage_1_start) = Self::first_ts_for_two_stages_seek(target_ts, window_ts) {
            // Make sure stage 1 includes target_ts
            let stage_1_end = segment.get_start() + gst::MSECOND;

            gst_info!(
                CAT,
                obj: pad,
                "segment {:?} starting 2 stages seek: 1st [{}, {}], 2d {}",
                segment_seqnum,
                stage_1_start,
                stage_1_end,
                target_ts,
            );

            state.seek = SeekState::InitSegment {
                expected_seqnum: segment_seqnum,
                remaining_streams: state.stream_count - 1,
                stage_1_start,
                stage_1_end,
                target_ts,
            };

            if PadStream::Audio == pad_stream {
                event
                    .make_mut()
                    .structure_mut()
                    .set(plugin::SegmentField::InitTwoStages.as_str(), &"");
            }
        } else {
            gst_debug!(
                CAT,
                obj: pad,
                "regular segment starting @ {} in Paused",
                target_ts,
            );
        }

        drop(state);
        pad.event_default(Some(bin), event)
    }

    fn send_stage_1_seek(
        &self,
        mut state: MutexGuard<State>,
        pad: &GhostPad,
        bin: &plugin::RendererBin,
        stage_1_start: ClockTime,
        stage_1_end: ClockTime,
        target_ts: ClockTime,
    ) {
        let seek_event = gst::event::Seek::new(
            1f64,
            gst::SeekFlags::ACCURATE | gst::SeekFlags::SEGMENT | gst::SeekFlags::FLUSH,
            gst::SeekType::Set,
            stage_1_start,
            gst::SeekType::Set,
            stage_1_end,
        );

        let seqnum = seek_event.get_seqnum();
        state.seek = SeekState::Stage1Segment {
            expected_seqnum: seqnum,
            remaining_streams: state.stream_count,
            // Wait for the segment to be received on the audio pad before accepting buffers
            accepts_audio_buffers: false,
            target_ts,
        };

        drop(state);

        gst_info!(
            CAT,
            obj: pad,
            "pushing stage 1 seek [{}, {}] {:?} (2d {})",
            stage_1_start,
            stage_1_end,
            seqnum,
            target_ts,
        );

        let sinkpad = self.audio_sinkpad.clone();
        bin.call_async(move |_| {
            sinkpad.push_event(seek_event);
        });
    }

    fn send_stage_2_seek(
        &self,
        mut state: MutexGuard<State>,
        bin: &plugin::RendererBin,
        target_ts: ClockTime,
    ) {
        let seek_event = gst::event::Seek::new(
            1f64,
            gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
            gst::SeekType::Set,
            target_ts,
            // FIXME restore the end defined prior to the 2 stages seek
            gst::SeekType::Set,
            ClockTime::none(),
        );

        let seqnum = seek_event.get_seqnum();
        state.seek = SeekState::Stage2Segment {
            expected_seqnum: seqnum,
            remaining_streams: state.stream_count,
        };

        drop(state);

        gst_info!(
            CAT,
            obj: bin,
            "stage 1 segment done, pushing stage 2 {} {:?}",
            target_ts,
            seqnum,
        );

        let sinkpad = self.audio_sinkpad.clone();
        bin.call_async(move |_| {
            sinkpad.push_event(seek_event);
        });
    }
}

/// Initialization.
impl RendererBin {
    fn new_queue(name: &str) -> gst::Element {
        let queue = gst::ElementFactory::make("queue2", Some(name)).unwrap();
        queue.set_property("max-size-bytes", &0u32).unwrap();
        queue.set_property("max-size-buffers", &0u32).unwrap();
        queue
            .set_property(
                "max-size-time",
                &plugin::renderer::DEFAULT_BUFFER_SIZE.as_u64(),
            )
            .unwrap();
        queue
    }

    fn setup_audio_elements(&self, bin: &plugin::RendererBin) {
        // Rendering elements
        let renderer_audioconvert =
            gst::ElementFactory::make("audioconvert", Some("renderer-audioconvert")).unwrap();

        let renderer_elements = &[&self.renderer_queue, &renderer_audioconvert, &self.renderer];

        bin.add_many(renderer_elements).unwrap();
        gst::Element::link_many(renderer_elements).unwrap();

        // Audio elements
        bin.add(&self.audio_queue).unwrap();

        // Audio tee
        bin.add(&self.audio_tee).unwrap();
        let audio_tee_renderer_src = self.audio_tee.get_request_pad("src_%u").unwrap();
        audio_tee_renderer_src
            .link(&self.renderer_queue_sinkpad)
            .unwrap();

        let audio_tee_audio_src = self.audio_tee.get_request_pad("src_%u").unwrap();
        audio_tee_audio_src
            .link(&self.audio_queue.get_static_pad("sink").unwrap())
            .unwrap();

        let mut elements = vec![&self.audio_tee, &self.audio_queue];
        elements.extend_from_slice(renderer_elements);

        for e in elements {
            e.sync_state_with_parent().unwrap();
        }
    }

    fn setup_video_elements(&self, bin: &plugin::RendererBin) {
        bin.add(&self.video_queue).unwrap();
        self.video_queue.sync_state_with_parent().unwrap();
    }

    fn setup_renderer(&self, bin: &plugin::RendererBin) {
        self.renderer
            .connect(
                plugin::renderer::SEGMENT_DONE_SIGNAL,
                true,
                clone!(@weak bin => move |args| {
                    let this = Self::from_instance(&bin);
                    let mut state = this.state.lock().unwrap();

                    use SeekState::*;
                    match &mut state.seek {
                        Stage1Segment { target_ts, .. } => {
                            let target_ts = *target_ts;
                            this.send_stage_2_seek(state, &bin, target_ts);
                        }
                        other => {
                            gst_debug!(CAT, obj: &bin, "renderer sent segment done in {:?}", other);
                        }
                    }

                    Option::None
                }),
            )
            .unwrap();

        // FIXME remove
        self.renderer
            .connect(
                plugin::renderer::MUST_REFRESH_SIGNAL,
                true,
                clone!(@weak bin => move |_| {
                    bin
                        .emit(plugin::renderer::MUST_REFRESH_SIGNAL, &[])
                        .unwrap()
                }),
            )
            .unwrap();
    }
}

impl ObjectSubclass for RendererBin {
    const NAME: &'static str = "MediaTocRendererBin";
    type Type = super::RendererBin;
    type ParentType = gst::Bin;
    type Instance = gst::subclass::ElementInstanceStruct<Self>;
    type Class = glib::subclass::simple::ClassStruct<Self>;

    glib_object_subclass!();

    fn with_class(klass: &glib::subclass::simple::ClassStruct<Self>) -> Self {
        let audio_sinkpad = GhostPad::builder_with_template(
            &klass.get_pad_template("audio_sink").unwrap(),
            Some("audio_sink"),
        )
        .chain_function(|pad, parent, buffer| {
            RendererBin::catch_panic_pad_function(
                parent,
                || Err(gst::FlowError::Error),
                |this, element| this.sink_chain(PadStream::Audio, pad, element, buffer),
            )
        })
        .event_function(|pad, parent, event| {
            RendererBin::catch_panic_pad_function(
                parent,
                || false,
                |this, element| this.sink_event(PadStream::Audio, pad, element, event),
            )
        })
        .build();

        let audio_queue = Self::new_queue("audio-queue");
        let audio_queue_sinkpad = audio_queue.get_static_pad("sink").unwrap();

        let video_sinkpad = GhostPad::builder_with_template(
            &klass.get_pad_template("video_sink").unwrap(),
            Some("video_sink"),
        )
        .chain_function(|pad, parent, buffer| {
            RendererBin::catch_panic_pad_function(
                parent,
                || Err(gst::FlowError::Error),
                |this, element| this.sink_chain(PadStream::Video, pad, element, buffer),
            )
        })
        .event_function(|pad, parent, event| {
            RendererBin::catch_panic_pad_function(
                parent,
                || false,
                |this, element| this.sink_event(PadStream::Video, pad, element, event),
            )
        })
        .build();

        // FIXME src pads of the bin should only be present if the matching sink pads are linked

        let renderer_queue = Self::new_queue("renderer-queue");
        let renderer_queue_sinkpad = renderer_queue.get_static_pad("sink").unwrap();

        let audio_srcpad = GhostPad::builder_with_template(
            &klass.get_pad_template("audio_src").unwrap(),
            Some("audio_src"),
        )
        .build();

        let video_srcpad = GhostPad::builder_with_template(
            &klass.get_pad_template("video_src").unwrap(),
            Some("video_src"),
        )
        .build();

        RendererBin {
            state: Arc::new(Mutex::new(State::default())),
            audio_sinkpad,
            audio_srcpad,
            renderer: gst::ElementFactory::make(plugin::renderer::NAME, Some("renderer")).unwrap(),
            renderer_queue_sinkpad,
            renderer_queue,
            audio_tee: gst::ElementFactory::make("tee", Some("renderer-audio-tee")).unwrap(),
            audio_queue,
            audio_queue_sinkpad,
            video_sinkpad,
            video_srcpad,
            video_queue: Self::new_queue("video-queue"),
        }
    }

    fn class_init(klass: &mut glib::subclass::simple::ClassStruct<Self>) {
        klass.set_metadata(
            "media-toc Audio Visualizer Renderer Bin",
            "Visualization",
            "Automates the construction of the elements required to render the media-toc Renderer",
            "François Laignel <fengalin@free.fr>",
        );

        let audio_caps = gst::ElementFactory::make("audioconvert", None)
            .unwrap()
            .get_static_pad("sink")
            .unwrap()
            .get_pad_template()
            .unwrap()
            .get_caps();

        let video_caps = gst::Caps::new_any();

        let audio_sinkpad_tmpl = gst::PadTemplate::new(
            "audio_sink",
            gst::PadDirection::Sink,
            gst::PadPresence::Request,
            &audio_caps,
        )
        .unwrap();
        klass.add_pad_template(audio_sinkpad_tmpl);

        let video_sinkpad_tmpl = gst::PadTemplate::new(
            "video_sink",
            gst::PadDirection::Sink,
            gst::PadPresence::Request,
            &video_caps,
        )
        .unwrap();
        klass.add_pad_template(video_sinkpad_tmpl);

        let audio_srcpad_tmpl = gst::PadTemplate::new(
            "audio_src",
            gst::PadDirection::Src,
            gst::PadPresence::Sometimes,
            &audio_caps,
        )
        .unwrap();
        klass.add_pad_template(audio_srcpad_tmpl);

        let video_srcpad_tmpl = gst::PadTemplate::new(
            "video_src",
            gst::PadDirection::Src,
            gst::PadPresence::Sometimes,
            &video_caps,
        )
        .unwrap();
        klass.add_pad_template(video_srcpad_tmpl);

        // FIXME this one could be avoided with a dedicated widget
        klass.add_signal(
            plugin::renderer::MUST_REFRESH_SIGNAL,
            glib::SignalFlags::RUN_LAST,
            &[],
            glib::types::Type::Unit,
        );

        klass.install_properties(PROPERTIES);
    }
}

impl ObjectImpl for RendererBin {
    fn set_property(&self, _bin: &plugin::RendererBin, id: usize, value: &glib::Value) {
        use glib::subclass::*;
        match PROPERTIES[id] {
            Property(plugin::renderer::DBL_RENDERER_IMPL_PROP, ..) => {
                self.renderer
                    .set_property(
                        plugin::renderer::DBL_RENDERER_IMPL_PROP,
                        value
                            .get_some::<&GBoxedDoubleRendererImpl>()
                            .expect("type checked upstream"),
                    )
                    .unwrap();
            }
            Property(plugin::renderer::CLOCK_REF_PROP, ..) => {
                self.renderer
                    .set_property(
                        plugin::renderer::CLOCK_REF_PROP,
                        &value
                            .get::<gst::Element>()
                            .expect("type checked upstream")
                            .expect("Value is None"),
                    )
                    .unwrap();
            }
            Property(plugin::renderer::BUFFER_SIZE_PROP, ..) => {
                let buffer_size = value.get_some::<u64>().expect("type checked upstream");
                self.renderer
                    .set_property(plugin::renderer::BUFFER_SIZE_PROP, &buffer_size)
                    .unwrap();
                self.renderer_queue
                    .set_property("max-size-time", &buffer_size)
                    .unwrap();
                self.audio_queue
                    .set_property("max-size-time", &buffer_size)
                    .unwrap();
                self.video_queue
                    .set_property("max-size-time", &buffer_size)
                    .unwrap();
            }
            _ => unimplemented!(),
        }
    }

    fn get_property(&self, _bin: &plugin::RendererBin, id: usize) -> glib::Value {
        match PROPERTIES[id] {
            glib::subclass::Property(plugin::renderer::DBL_RENDERER_IMPL_PROP, ..) => self
                .renderer
                .get_property(plugin::renderer::DBL_RENDERER_IMPL_PROP)
                .unwrap(),
            _ => unimplemented!(),
        }
    }

    fn constructed(&self, bin: &plugin::RendererBin) {
        self.parent_constructed(bin);

        self.setup_audio_elements(bin);

        self.audio_sinkpad
            .set_target(Some(&self.audio_tee.get_static_pad("sink").unwrap()))
            .unwrap();
        bin.add_pad(&self.audio_sinkpad).unwrap();

        self.audio_srcpad
            .set_target(Some(&self.audio_queue.get_static_pad("src").unwrap()))
            .unwrap();
        bin.add_pad(&self.audio_srcpad).unwrap();

        self.setup_renderer(bin);

        self.setup_video_elements(bin);

        self.video_sinkpad
            .set_target(Some(&self.video_queue.get_static_pad("sink").unwrap()))
            .unwrap();
        bin.add_pad(&self.video_sinkpad).unwrap();

        self.video_srcpad
            .set_target(Some(&self.video_queue.get_static_pad("src").unwrap()))
            .unwrap();
        bin.add_pad(&self.video_srcpad).unwrap();
    }
}

/// State change
impl RendererBin {
    fn prepare(&self, bin: &plugin::RendererBin) -> Result<(), gst::ErrorMessage> {
        gst_debug!(CAT, obj: bin, "Preparing");
        self.state.lock().unwrap().playback = PlaybackState::Prepared;
        gst_debug!(CAT, obj: bin, "Prepared");
        Ok(())
    }

    fn unprepare(&self, bin: &plugin::RendererBin) {
        gst_debug!(CAT, obj: bin, "Unpreparing");
        self.state.lock().unwrap().playback = PlaybackState::Unprepared;
        gst_debug!(CAT, obj: bin, "Unprepared");
    }

    fn stop(&self, bin: &plugin::RendererBin) -> Result<(), gst::ErrorMessage> {
        gst_debug!(CAT, obj: bin, "Stopping");
        self.state.lock().unwrap().playback = PlaybackState::Stopped;
        gst_debug!(CAT, obj: bin, "Stopped");
        Ok(())
    }

    fn play(&self, bin: &plugin::RendererBin) -> Result<(), gst::ErrorMessage> {
        gst_debug!(CAT, obj: bin, "Starting");
        self.state.lock().unwrap().playback = PlaybackState::Playing;
        gst_debug!(CAT, obj: bin, "Started");
        Ok(())
    }

    fn pause(&self, bin: &plugin::RendererBin) -> Result<(), gst::ErrorMessage> {
        gst_debug!(CAT, obj: bin, "Pausing");
        {
            let mut state = self.state.lock().unwrap();
            state.playback = PlaybackState::Paused;
            state.seek = SeekState::None;
        }
        gst_debug!(CAT, obj: bin, "Paused");
        Ok(())
    }
}

impl ElementImpl for RendererBin {
    fn change_state(
        &self,
        bin: &plugin::RendererBin,
        transition: gst::StateChange,
    ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
        gst_trace!(CAT, obj: bin, "Changing state {:?}", transition);

        match transition {
            gst::StateChange::NullToReady => {
                self.prepare(bin).map_err(|err| {
                    bin.post_error_message(err);
                    gst::StateChangeError
                })?;
            }
            gst::StateChange::PlayingToPaused => {
                self.pause(bin).map_err(|_| gst::StateChangeError)?;
            }
            gst::StateChange::ReadyToNull => {
                self.unprepare(bin);
            }
            _ => (),
        }

        let success = self.parent_change_state(bin, transition)?;

        match transition {
            gst::StateChange::PausedToPlaying => {
                self.play(bin).map_err(|_| gst::StateChangeError)?;
            }
            gst::StateChange::PausedToReady => {
                self.stop(bin).map_err(|_| gst::StateChangeError)?;
            }
            _ => (),
        }

        Ok(success)
    }
}

impl BinImpl for RendererBin {}
