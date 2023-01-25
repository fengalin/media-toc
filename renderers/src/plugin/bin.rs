use gst::{
    self,
    glib::{self, clone, subclass::Signal},
    prelude::*,
    subclass::prelude::*,
    ClockTime, GhostPad, Seqnum,
};

use once_cell::sync::Lazy;

use std::{
    ops::Deref,
    sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard},
};

use crate::{
    generic::{GBoxedDoubleRendererImpl, WindowTimestamps},
    plugin,
};

pub const NAME: &str = "mediatocrendererbin";

static CAT: Lazy<gst::DebugCategory> = Lazy::new(|| {
    gst::DebugCategory::new(
        NAME,
        gst::DebugColorFlags::empty(),
        Some("media-toc Renderer Bin"),
    )
});

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
    Uncontrolled,
    PlayRange {
        expected_seqnum: Seqnum,
        remaining_streams: usize,
        ts_to_restore: ClockTime,
        stop_to_restore: Option<ClockTime>,
    },
    PlayRangeRestore {
        expected_seqnum: Seqnum,
    },
    Stage1 {
        expected_seqnum: Seqnum,
        accepts_audio_buffers: bool,
        target_ts: ClockTime,
        stop_to_restore: Option<ClockTime>,
    },
    Stage2 {
        expected_seqnum: Seqnum,
        remaining_streams: usize,
        accepts_video_buffers: bool,
    },
}

impl Default for SeekState {
    fn default() -> Self {
        SeekState::Uncontrolled
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Debug, Default)]
struct State {
    playback: PlaybackState,
    seek: SeekState,
    stream_count: usize,
}

#[derive(Debug, Default)]
struct RendererInit {
    dbl_renderer_impl: Option<GBoxedDoubleRendererImpl>,
    clock_ref: Option<gst::Element>,
}

#[derive(Debug)]
struct AudioPipeline {
    sinkpad: GhostPad,
    renderer: gst::Element,
    renderer_queue_sinkpad: gst::Pad,
    renderer_queue: gst::Element,
    audio_queue: gst::Element,
    audio_queue_sinkpad: gst::Pad,
    clock_ref: gst::Element,
}

#[derive(Debug)]
enum Audio {
    Uninitialized(RendererInit),
    Initialized(AudioPipeline),
}

impl Default for Audio {
    fn default() -> Self {
        Audio::Uninitialized(RendererInit::default())
    }
}

struct AudioGuard<'a>(RwLockReadGuard<'a, Audio>);

impl Deref for AudioGuard<'_> {
    type Target = AudioPipeline;

    #[track_caller]
    fn deref(&self) -> &Self::Target {
        match *self.0 {
            Audio::Initialized(ref audio) => audio,
            _ => panic!("AudioPipeline not initialized"),
        }
    }
}

#[derive(Debug)]
struct VideoPipeline {
    sinkpad: GhostPad,
    queue: gst::Element,
}

struct VideoGuard<'a>(RwLockReadGuard<'a, Option<VideoPipeline>>);

impl Deref for VideoGuard<'_> {
    type Target = VideoPipeline;

    #[track_caller]
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("VideoPipeline not initialized")
    }
}

#[derive(Debug, Default)]
pub struct RendererBin {
    state: Arc<Mutex<State>>,
    audio: RwLock<Audio>,
    video: RwLock<Option<VideoPipeline>>,
}

impl RendererBin {
    #[track_caller]
    fn audio(&self) -> AudioGuard<'_> {
        AudioGuard(self.audio.read().unwrap())
    }

    #[track_caller]
    fn video(&self) -> VideoGuard<'_> {
        VideoGuard(self.video.read().unwrap())
    }
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
        buffer: gst::Buffer,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        let state = self.state.lock().unwrap();
        match state.seek {
            SeekState::Stage1 {
                accepts_audio_buffers,
                ..
            } => {
                if PadStream::Audio == pad_stream && accepts_audio_buffers {
                    drop(state);
                    return self.audio().renderer_queue_sinkpad.chain(buffer);
                }

                return Ok(gst::FlowSuccess::Ok);
            }
            SeekState::Stage2 {
                accepts_video_buffers,
                ..
            } => {
                if PadStream::Video == pad_stream && !accepts_video_buffers {
                    return Ok(gst::FlowSuccess::Ok);
                }
            }
            _ => (),
        }
        drop(state);

        gst::ProxyPad::chain_default(pad, Some(&*self.obj()), buffer)
    }

    fn sink_event(&self, pad_stream: PadStream, pad: &GhostPad, mut event: gst::Event) -> bool {
        use gst::EventView::*;
        use SeekState::*;

        match event.view() {
            StreamCollection(evt) => {
                // FIXME only retain actually selected streams among Video & Audio
                self.state.lock().unwrap().stream_count = evt.stream_collection().len();
            }
            FlushStart(_) | FlushStop(_) => {
                if let Stage2 {
                    expected_seqnum, ..
                }
                | PlayRange {
                    expected_seqnum, ..
                }
                | PlayRangeRestore { expected_seqnum } = self.state.lock().unwrap().seek
                {
                    // Don't flush the renderer queue
                    if PadStream::Audio == pad_stream && expected_seqnum == event.seqnum() {
                        return self.audio().audio_queue_sinkpad.send_event(event);
                    }
                    // else forward and wait for the Segment before deciding what to do
                }
            }
            Segment(_) => {
                let mut state = self.state.lock().unwrap();
                let seqnum = event.seqnum();

                match &mut state.seek {
                    Uncontrolled => (),
                    PlayRange {
                        expected_seqnum,
                        remaining_streams,
                        ..
                    } => {
                        if seqnum != *expected_seqnum {
                            return self.unexpected_segment(state, pad_stream, pad, event);
                        }

                        if PadStream::Audio == pad_stream {
                            event
                                .make_mut()
                                .structure_mut()
                                .set(plugin::SegmentField::PlayRange.as_str(), &true);
                        }

                        *remaining_streams -= 1;
                        if *remaining_streams == 0 {
                            gst::debug!(CAT, obj: pad, "Got all play range Segments {:?}", seqnum);

                            drop(state);
                            let bin = self.obj();
                            let ret = gst::Pad::event_default(pad, Some(&*bin), event);
                            if ret {
                                bin.parent_call_async(move |_bin, parent| {
                                    let _ = parent.set_state(gst::State::Playing);
                                });
                            }

                            return ret;
                        } else {
                            gst::debug!(CAT, obj: pad, "Got play range Segment {:?}", seqnum);
                        }
                    }
                    PlayRangeRestore { expected_seqnum } => {
                        if seqnum != *expected_seqnum {
                            return self.unexpected_segment(state, pad_stream, pad, event);
                        }

                        if PadStream::Audio == pad_stream {
                            event
                                .make_mut()
                                .structure_mut()
                                .set(plugin::SegmentField::RestoringPosition.as_str(), &true);
                            state.seek = Uncontrolled;
                            self.obj()
                                .emit_by_name::<()>(plugin::PLAY_RANGE_DONE_SIGNAL, &[]);
                        }

                        gst::debug!(
                            CAT,
                            obj: pad,
                            "Got play range restoring Segment {:?}",
                            seqnum
                        );
                    }
                    Stage1 {
                        expected_seqnum,
                        accepts_audio_buffers,
                        ..
                    } => {
                        if seqnum != *expected_seqnum {
                            return self.unexpected_segment(state, pad_stream, pad, event);
                        }

                        if PadStream::Audio == pad_stream {
                            gst::debug!(CAT, obj: pad, "forwarding stage 1 segment {:?}", seqnum);

                            *accepts_audio_buffers = true;
                            event
                                .make_mut()
                                .structure_mut()
                                .set(plugin::SegmentField::Stage1.as_str(), &true);

                            drop(state);
                            let ret = self.audio().renderer_queue_sinkpad.send_event(event);
                            if !ret {
                                gst::error!(
                                    CAT,
                                    obj: pad,
                                    "failed to forward stage 1 segment {:?}",
                                    seqnum,
                                );
                                self.state.lock().unwrap().seek = Uncontrolled;
                            }

                            return true;
                        } else {
                            gst::debug!(CAT, obj: pad, "filtering stage 1 segment {:?}", seqnum);
                            return true;
                        }
                    }
                    Stage2 {
                        expected_seqnum,
                        remaining_streams,
                        accepts_video_buffers,
                    } => {
                        if seqnum != *expected_seqnum {
                            return self.unexpected_segment(state, pad_stream, pad, event);
                        }

                        if PadStream::Audio == pad_stream {
                            event
                                .make_mut()
                                .structure_mut()
                                .set(plugin::SegmentField::Stage2.as_str(), &true);
                        }

                        *remaining_streams -= 1;
                        if *remaining_streams == 0 {
                            gst::debug!(CAT, obj: pad, "got all stage 2 segments {:?}", seqnum);

                            state.seek = Uncontrolled;
                        } else {
                            gst::debug!(CAT, obj: pad, "got stage 2 segment {:?}", seqnum);

                            if PadStream::Video == pad_stream {
                                *accepts_video_buffers = true;
                            }
                        }
                    }
                }
            }
            SegmentDone(_) => {
                // FIXME can't send stage 2 seek as soon as SegmentDone is received
                // by bin since it takes time for the buffers to reach the renderer
                // meaning that buffers can be flushed before they reach the renderer.

                let seqnum = event.seqnum();
                let mut state = self.state.lock().unwrap();

                if let Stage1 {
                    expected_seqnum, ..
                } = state.seek
                {
                    if seqnum == expected_seqnum {
                        if PadStream::Audio == pad_stream {
                            gst::debug!(
                                CAT,
                                obj: pad,
                                "forwarding stage 1 SegmentDone {:?}",
                                seqnum
                            );

                            drop(state);
                            let ret = self.audio().renderer_queue_sinkpad.send_event(event);
                            if !ret {
                                gst::error!(
                                    CAT,
                                    obj: pad,
                                    "failed to forward SegmentDone {:?}",
                                    seqnum
                                );
                                self.state.lock().unwrap().seek = Uncontrolled;
                            }

                            return ret;
                        } else {
                            gst::debug!(
                                CAT,
                                obj: pad,
                                "filtering stage 1 SegmentDone {:?}",
                                seqnum
                            );
                        };

                        return true;
                    }

                    gst::debug!(CAT, obj: pad, "unexpected SegmentDone {:?}", seqnum);
                    state.seek = Uncontrolled;
                }
            }
            _ => (),
        }

        gst::Pad::event_default(pad, Some(&*self.obj()), event)
    }
}

/// Src Pads handlers.
impl RendererBin {
    fn src_event(&self, pad: &GhostPad, bin: &plugin::RendererBin, event: gst::Event) -> bool {
        use gst::EventView::*;

        // FIXME there are many Qos overflow showing up on Video Stream while playing
        /*
        if let gst::EventView::Qos(evt) = event.view() {
            println!("{:?} {:?}", pad_stream, evt);
        }
        */

        if let Seek(_) = event.view() {
            return self.handle_seek(pad, bin, event);
        }

        gst::Pad::event_default(pad, Some(bin), event)
    }
}

/// Seek handler.
impl RendererBin {
    fn handle_seek(
        &self,
        pad: &GhostPad,
        bin: &plugin::RendererBin,
        mut event: gst::Event,
    ) -> bool {
        use SeekState::*;

        let seqnum = event.seqnum();

        let (_rate, _flags, start_type, start, stop_type, stop) = match event.view() {
            gst::EventView::Seek(seek) => seek.get(),
            evt => panic!("Unexpected {:?} in handle_seek", evt),
        };

        if matches!(*self.audio.read().unwrap(), Audio::Uninitialized(_)) {
            gst::debug!(
                CAT,
                obj: pad,
                "No audio => forwading seek to {} {:?}",
                start,
                seqnum,
            );
            return false;
        }

        let mut state = self.state.lock().unwrap();

        match start_type {
            gst::SeekType::Set => (),
            other => {
                gst::debug!(CAT, obj: pad, "seek start type {:?} {:?}", other, seqnum);
                drop(state);
                return gst::Pad::event_default(pad, Some(bin), event);
            }
        }

        let target_ts: ClockTime = match TryFrom::try_from(start) {
            Ok(Some(start)) => start,
            other => {
                if other.is_err() {
                    gst::debug!(
                        CAT,
                        obj: pad,
                        "not Time seek start {:?} {:?}",
                        start,
                        seqnum
                    );
                } else {
                    gst::debug!(CAT, obj: pad, "seek start is None {:?}", seqnum);
                }
                drop(state);
                return gst::Pad::event_default(pad, Some(bin), event);
            }
        };

        let stop_to_restore = match stop_type {
            gst::SeekType::Set => pad
                .sticky_event::<gst::event::Segment<_>>(0)
                .and_then(|evt| match evt.segment().stop() {
                    gst::GenericFormattedValue::Time(opt_ct) => opt_ct,
                    _ => None,
                }),
            _ => None,
        };

        let stop: Option<ClockTime> = match TryFrom::try_from(stop) {
            Ok(stop) => stop,
            _err => {
                gst::debug!(CAT, obj: pad, "not Time seek stop {:?} {:?}", stop, seqnum);
                drop(state);
                return gst::Pad::event_default(pad, Some(bin), event);
            }
        };

        let is_incoming_play_range = event.structure().map_or(false, |evt_struct| {
            evt_struct.has_field(plugin::SeekField::PlayRange.as_str())
        });

        match state.seek {
            Stage1 {
                expected_seqnum, ..
            }
            | Stage2 {
                expected_seqnum, ..
            } if expected_seqnum == seqnum => {
                gst::debug!(CAT, obj: pad, "Filtering additional seek {:?}", seqnum);
                drop(state);
                return true;
            }
            PlayRange {
                expected_seqnum, ..
            } => {
                if expected_seqnum == seqnum {
                    gst::debug!(CAT, obj: pad, "Filtering additional seek {:?}", seqnum);
                    drop(state);
                    return true;
                } else if !is_incoming_play_range {
                    // Let play range complete before submitting
                    // a regular seek so as to keep state management easy.
                    gst::warning!(
                        CAT,
                        obj: pad,
                        "Play range: reject regular seek {:?}",
                        seqnum
                    );
                    return false;
                }
            }
            _ => (),
        }

        let window_ts = self
            .audio()
            .renderer
            .emit_by_name::<Option<WindowTimestamps>>(plugin::GET_WINDOW_TIMESTAMPS_SIGNAL, &[]);

        let window_ts = match window_ts {
            Some(window_ts) => window_ts,
            None => {
                state.seek = Uncontrolled;
                gst::debug!(
                    CAT,
                    obj: pad,
                    "unknown rendering conditions for Seek starting @ {}",
                    target_ts,
                );
                drop(state);
                return gst::Pad::event_default(pad, Some(bin), event);
            }
        };

        if is_incoming_play_range {
            event = self.prepare_play_range_seek(
                &mut state,
                pad,
                seqnum,
                target_ts,
                window_ts.end,
                stop_to_restore,
            );

            drop(state);
            return gst::Pad::event_default(pad, Some(bin), event);
        }

        if state.playback.is_playing() {
            gst::debug!(
                CAT,
                obj: pad,
                "Seek [{}, {}] playing {:?}",
                target_ts,
                stop.display(),
                seqnum,
            );
            drop(state);
            return gst::Pad::event_default(pad, Some(bin), event);
        }

        let stage_1_start = target_ts.saturating_sub(window_ts.range / 2);
        let stage_1_end = target_ts + ClockTime::MSECOND;

        gst::info!(
            CAT,
            obj: pad,
            "Seek {:?} starting 2 stages seek: 1st [{}, {}], 2d {}",
            seqnum,
            stage_1_start,
            stage_1_end,
            target_ts,
        );
        event = gst::event::Seek::builder(
            1f64,
            gst::SeekFlags::ACCURATE | gst::SeekFlags::SEGMENT | gst::SeekFlags::FLUSH,
            gst::SeekType::Set,
            stage_1_start,
            gst::SeekType::Set,
            stage_1_end,
        )
        .seqnum(seqnum)
        .build();

        state.seek = Stage1 {
            expected_seqnum: seqnum,
            // Wait for the segment to be received on the audio pad before accepting buffers
            accepts_audio_buffers: false,
            target_ts,
            stop_to_restore,
        };

        drop(state);
        gst::Pad::event_default(pad, Some(bin), event)
    }

    fn unexpected_segment(
        &self,
        state: MutexGuard<State>,
        pad_stream: PadStream,
        pad: &GhostPad,
        event: gst::Event,
    ) -> bool {
        gst::warning!(
            CAT,
            obj: pad,
            "{:?}: got {:?} with {:?} on {:?}",
            state.seek,
            event.type_(),
            event.seqnum(),
            pad_stream,
        );

        gst::Pad::event_default(pad, Some(&*self.obj()), event)
    }

    fn prepare_play_range_seek(
        &self,
        state: &mut MutexGuard<State>,
        pad: &GhostPad,
        seqnum: Seqnum,
        target_ts: ClockTime,
        window_end: Option<ClockTime>,
        stop_to_restore: Option<ClockTime>,
    ) -> gst::Event {
        use SeekState::*;

        let stream_count = state.stream_count;

        if let PlayRange {
            expected_seqnum,
            remaining_streams,
            ts_to_restore,
            stop_to_restore,
        } = &mut state.seek
        {
            gst::debug!(
                CAT,
                obj: pad,
                "Updating play range [{}, {}] final [{}, {}] {:?}",
                target_ts,
                window_end.display(),
                ts_to_restore,
                stop_to_restore.display(),
                seqnum,
            );

            *expected_seqnum = seqnum;
            *remaining_streams = stream_count;
        } else {
            let mut position_query = gst::query::Position::new(gst::Format::Time);
            self.audio().clock_ref.query(&mut position_query);
            let ts_to_restore = match position_query.result() {
                gst::GenericFormattedValue::Time(opt_ct) => opt_ct.unwrap(),
                other => unreachable!("got {:?}", other),
            };

            gst::debug!(
                CAT,
                obj: pad,
                "Seek [{}, {}] for range playing final [{}, {}] {:?}",
                target_ts,
                window_end.display(),
                ts_to_restore,
                stop_to_restore.display(),
                seqnum,
            );

            state.seek = PlayRange {
                expected_seqnum: seqnum,
                remaining_streams: stream_count,
                ts_to_restore,
                stop_to_restore,
            };
        }

        gst::event::Seek::builder(
            1f64,
            gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH | gst::SeekFlags::SEGMENT,
            gst::SeekType::Set,
            Some(target_ts),
            gst::SeekType::Set,
            window_end,
        )
        .seqnum(seqnum)
        .build()
    }

    fn send_stage_2_seek(
        &self,
        mut state: MutexGuard<State>,
        bin: &plugin::RendererBin,
        target_ts: ClockTime,
        stop_to_restore: Option<ClockTime>,
    ) {
        let seek_event = gst::event::Seek::new(
            1f64,
            gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
            gst::SeekType::Set,
            Some(target_ts),
            gst::SeekType::Set,
            stop_to_restore,
        );

        let seqnum = seek_event.seqnum();
        state.seek = SeekState::Stage2 {
            expected_seqnum: seqnum,
            remaining_streams: state.stream_count,
            accepts_video_buffers: false,
        };

        gst::debug!(
            CAT,
            obj: bin,
            "stage 1 segment handled, pushing stage 2 {} {:?}",
            target_ts,
            seqnum,
        );
        let sinkpad = self.audio().sinkpad.clone();

        bin.call_async(move |_| {
            sinkpad.push_event(seek_event);
        });
    }

    fn in_sync_probe(bin: plugin::RendererBin, info: &gst::PadProbeInfo) {
        if let Some(gst::PadProbeData::Event(event)) = info.data.as_ref() {
            if let gst::EventView::SegmentDone(_) = event.view() {
                gst::debug!(CAT, obj: &bin, "Got SegmentDone from the in-sync stream");

                let imp = bin.imp();
                let mut state = imp.state.lock().unwrap();
                if let SeekState::PlayRange {
                    ts_to_restore,
                    stop_to_restore,
                    ..
                } = state.seek
                {
                    let seek = gst::event::Seek::new(
                        1f64,
                        gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
                        gst::SeekType::Set,
                        Some(ts_to_restore),
                        gst::SeekType::Set,
                        stop_to_restore,
                    );

                    state.seek = SeekState::PlayRangeRestore {
                        expected_seqnum: seek.seqnum(),
                    };

                    let sinkpad = imp.audio().sinkpad.clone();
                    bin.parent_call_async(move |bin, parent| {
                        if parent.set_state(gst::State::Paused).is_ok() {
                            gst::debug!(
                                CAT,
                                obj: bin,
                                "Play range: restoring position with seek {:?}",
                                seek.seqnum(),
                            );

                            sinkpad.push_event(seek);
                        }
                    });
                }
            }
        }
    }
}

/// Initialization.
impl RendererBin {
    fn new_queue(name: &str) -> gst::Element {
        gst::ElementFactory::make("queue2")
            .name(name)
            .property("max-size-bytes", &0u32)
            .property("max-size-buffers", &0u32)
            .property(
                "max-size-time",
                &plugin::renderer::DEFAULT_BUFFER_SIZE.as_u64(),
            )
            .build()
            .unwrap()
    }

    fn setup_audio_pipeline(&self) {
        let mut audio = self.audio.write().unwrap();
        let renderer_init = match *audio {
            Audio::Uninitialized(ref renderer_init) => renderer_init,
            _ => panic!("AudioPipeline already initialized"),
        };

        let tee = gst::ElementFactory::make("tee")
            .name("renderer-audio-tee")
            .build()
            .unwrap();

        let renderer_queue = Self::new_queue("renderer-queue");
        let renderer_queue_sinkpad = renderer_queue.static_pad("sink").unwrap();

        let audio_queue = Self::new_queue("audio-queue");
        let audio_queue_sinkpad = audio_queue.static_pad("sink").unwrap();

        // Rendering elements
        let renderer_audioconvert = gst::ElementFactory::make("audioconvert")
            .name("renderer-audioconvert")
            .build()
            .unwrap();

        let renderer = gst::ElementFactory::make(plugin::renderer::NAME)
            .name("renderer")
            .build()
            .unwrap();

        let renderer_elements = &[&renderer_queue, &renderer_audioconvert, &renderer];

        let bin = self.obj();
        bin.add_many(renderer_elements).unwrap();
        gst::Element::link_many(renderer_elements).unwrap();

        // Audio elements
        bin.add(&audio_queue).unwrap();

        // Audio tee
        bin.add(&tee).unwrap();
        let tee_renderer_src = tee.request_pad_simple("src_%u").unwrap();
        tee_renderer_src.link(&renderer_queue_sinkpad).unwrap();

        let tee_audio_src = tee.request_pad_simple("src_%u").unwrap();
        tee_audio_src
            .link(&audio_queue.static_pad("sink").unwrap())
            .unwrap();

        let mut elements = vec![&tee, &audio_queue];
        elements.extend_from_slice(renderer_elements);

        // GhostPads
        let sinkpad = GhostPad::builder_with_template(
            &bin.pad_template("audio_sink").unwrap(),
            Some("audio_sink"),
        )
        .chain_function(|pad, parent, buffer| {
            RendererBin::catch_panic_pad_function(
                parent,
                || Err(gst::FlowError::Error),
                |this| this.sink_chain(PadStream::Audio, pad, buffer),
            )
        })
        .event_function(|pad, parent, event| {
            RendererBin::catch_panic_pad_function(
                parent,
                || false,
                |this| this.sink_event(PadStream::Audio, pad, event),
            )
        })
        .build();

        sinkpad
            .set_target(Some(&tee.static_pad("sink").unwrap()))
            .unwrap();
        bin.add_pad(&sinkpad).unwrap();

        let srcpad = GhostPad::builder_with_template(
            &bin.pad_template("audio_src").unwrap(),
            Some("audio_src"),
        )
        .event_function(|pad, parent, event| {
            RendererBin::catch_panic_pad_function(
                parent,
                || false,
                |this| this.src_event(pad, &this.obj(), event),
            )
        })
        .build();

        let audio_queue_srcpad = audio_queue.static_pad("src").unwrap();
        srcpad.set_target(Some(&audio_queue_srcpad)).unwrap();
        bin.add_pad(&srcpad).unwrap();

        audio_queue_srcpad
            .add_probe(
                gst::PadProbeType::EVENT_DOWNSTREAM,
                glib::clone!(@weak bin => @default-panic, move |_pad, info| {
                    Self::in_sync_probe(bin, info);
                    gst::PadProbeReturn::Ok
                }),
            )
            .unwrap();

        renderer.set_property(
            plugin::renderer::DBL_RENDERER_IMPL_PROP,
            renderer_init.dbl_renderer_impl.as_ref().unwrap(),
        );

        let clock_ref = renderer_init.clock_ref.as_ref().unwrap().clone();
        renderer.set_property(plugin::renderer::CLOCK_REF_PROP, &clock_ref);

        renderer.connect(
            plugin::renderer::SEGMENT_DONE_SIGNAL,
            true,
            clone!(@weak bin => @default-panic, move |_args| {
                let mut state = bin.imp().state.lock().unwrap();

                use SeekState::*;
                match &mut state.seek {
                    Stage1 { target_ts, stop_to_restore, .. } => {
                        let target_ts = *target_ts;
                        let stop_to_restore = *stop_to_restore;
                        bin.imp().send_stage_2_seek(state, &bin, target_ts, stop_to_restore);
                    }
                    other => {
                        gst::debug!(CAT, obj: &bin, "renderer sent segment done in {:?}", other);
                    }
                }

                Option::None
            }),
        );

        // FIXME remove
        renderer.connect(
            plugin::renderer::MUST_REFRESH_SIGNAL,
            true,
            clone!(@weak bin => @default-panic, move |_| {
                bin.emit_by_name::<()>(plugin::renderer::MUST_REFRESH_SIGNAL, &[]);
                None
            }),
        );

        *audio = Audio::Initialized(AudioPipeline {
            sinkpad,
            renderer,
            renderer_queue_sinkpad,
            renderer_queue,
            audio_queue,
            audio_queue_sinkpad,
            clock_ref,
        });
    }

    fn setup_video_pipeline(&self) {
        let bin = self.obj();

        let mut video = self.video.write().unwrap();
        assert!(video.is_none(), "VideoPipeline already initialized");

        let queue = Self::new_queue("video-queue");
        bin.add(&queue).unwrap();

        let sinkpad = GhostPad::builder_with_template(
            &bin.pad_template("video_sink").unwrap(),
            Some("video_sink"),
        )
        .chain_function(|pad, parent, buffer| {
            RendererBin::catch_panic_pad_function(
                parent,
                || Err(gst::FlowError::Error),
                |this| this.sink_chain(PadStream::Video, pad, buffer),
            )
        })
        .event_function(|pad, parent, event| {
            RendererBin::catch_panic_pad_function(
                parent,
                || false,
                |this| this.sink_event(PadStream::Video, pad, event),
            )
        })
        .build();

        sinkpad
            .set_target(Some(&queue.static_pad("sink").unwrap()))
            .unwrap();
        bin.add_pad(&sinkpad).unwrap();

        let srcpad = GhostPad::builder_with_template(
            &bin.pad_template("video_src").unwrap(),
            Some("video_src"),
        )
        .event_function(|pad, parent, event| {
            RendererBin::catch_panic_pad_function(
                parent,
                || false,
                |this| this.src_event(pad, &this.obj(), event),
            )
        })
        .build();

        srcpad
            .set_target(Some(&queue.static_pad("src").unwrap()))
            .unwrap();
        bin.add_pad(&srcpad).unwrap();

        *video = Some(VideoPipeline { sinkpad, queue });
    }
}

#[glib::object_subclass]
impl ObjectSubclass for RendererBin {
    const NAME: &'static str = "MediaTocRendererBin";
    type Type = super::RendererBin;
    type ParentType = gst::Bin;
}

impl ObjectImpl for RendererBin {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
            vec![
                glib::ParamSpecBoxed::new(
                    plugin::renderer::DBL_RENDERER_IMPL_PROP,
                    "Double Renderer",
                    "Implementation for the Double Renderer",
                    GBoxedDoubleRendererImpl::static_type(),
                    glib::ParamFlags::READWRITE,
                ),
                glib::ParamSpecObject::new(
                    plugin::renderer::CLOCK_REF_PROP,
                    "Clock reference",
                    "Element providing the clock reference",
                    gst::Element::static_type(),
                    glib::ParamFlags::WRITABLE,
                ),
                // FIXME use a ClockTime
                glib::ParamSpecUInt64::new(
                    plugin::renderer::BUFFER_SIZE_PROP,
                    "Renderer Size (ns)",
                    "Internal buffer size in ns",
                    1_000u64,
                    u64::MAX,
                    plugin::renderer::DEFAULT_BUFFER_SIZE.as_u64(),
                    glib::ParamFlags::WRITABLE,
                ),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        match pspec.name() {
            plugin::renderer::DBL_RENDERER_IMPL_PROP => {
                let mut audio = self.audio.write().unwrap();
                match *audio {
                    Audio::Uninitialized(ref mut renderer_init) => {
                        renderer_init.dbl_renderer_impl = Some(
                            value
                                .get::<&GBoxedDoubleRendererImpl>()
                                .expect("type checked upstream")
                                .clone(),
                        );
                    }
                    _ => panic!("AudioPipeline already initialized"),
                }
            }
            plugin::renderer::CLOCK_REF_PROP => {
                let mut audio = self.audio.write().unwrap();
                match *audio {
                    Audio::Uninitialized(ref mut renderer_init) => {
                        renderer_init.clock_ref =
                            Some(value.get::<gst::Element>().expect("type checked upstream"));
                    }
                    _ => panic!("AudioPipeline already initialized"),
                }
            }
            plugin::renderer::BUFFER_SIZE_PROP => {
                let buffer_size = value.get::<u64>().expect("type checked upstream");
                if let Audio::Initialized(ref audio) = *self.audio.read().unwrap() {
                    audio
                        .renderer
                        .set_property(plugin::renderer::BUFFER_SIZE_PROP, &buffer_size);
                    audio
                        .renderer_queue
                        .set_property("max-size-time", &buffer_size);
                    audio
                        .audio_queue
                        .set_property("max-size-time", &buffer_size);
                }

                if let Some(ref video) = *self.video.read().unwrap() {
                    video.queue.set_property("max-size-time", &buffer_size);
                }
            }
            _ => unimplemented!(),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            plugin::renderer::DBL_RENDERER_IMPL_PROP => match *self.audio.read().unwrap() {
                Audio::Initialized(ref audio) => audio
                    .renderer
                    .property(plugin::renderer::DBL_RENDERER_IMPL_PROP),
                _ => GBoxedDoubleRendererImpl::none().to_value(),
            },
            _ => unimplemented!(),
        }
    }

    fn signals() -> &'static [Signal] {
        static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
            vec![
                Signal::builder(plugin::PLAY_RANGE_DONE_SIGNAL)
                    .run_last()
                    .build(),
                // FIXME this one could be avoided with a dedicated widget
                Signal::builder(plugin::MUST_REFRESH_SIGNAL)
                    .run_last()
                    .build(),
            ]
        });

        SIGNALS.as_ref()
    }
}

/// State change
impl RendererBin {
    fn prepare(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp: self, "Preparing");
        let mut state = self.state.lock().unwrap();
        state.playback = PlaybackState::Prepared;
        gst::debug!(CAT, imp: self, "Prepared");
        Ok(())
    }

    fn unprepare(&self) {
        gst::debug!(CAT, imp: self, "Unpreparing");
        let mut state = self.state.lock().unwrap();
        state.playback = PlaybackState::Unprepared;
        gst::debug!(CAT, imp: self, "Unprepared");
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp: self, "Stopping");
        let mut state = self.state.lock().unwrap();
        state.playback = PlaybackState::Stopped;
        state.seek = SeekState::Uncontrolled;
        gst::debug!(CAT, imp: self, "Stopped");
        Ok(())
    }

    fn play(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp: self, "Starting");
        let mut state = self.state.lock().unwrap();
        state.playback = PlaybackState::Playing;
        gst::debug!(CAT, imp: self, "Started");
        Ok(())
    }

    fn pause(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp: self, "Pausing");
        let mut state = self.state.lock().unwrap();
        state.playback = PlaybackState::Paused;
        gst::debug!(CAT, imp: self, "Paused");
        Ok(())
    }
}

impl GstObjectImpl for RendererBin {}

impl ElementImpl for RendererBin {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: Lazy<gst::subclass::ElementMetadata> = Lazy::new(|| {
            gst::subclass::ElementMetadata::new(
                "media-toc Audio Visualizer Renderer Bin",
                "Visualization",
                "Automates the construction of the elements required to render the media-toc Renderer",
                "François Laignel <fengalin@free.fr>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static PAD_TEMPLATES: Lazy<Vec<gst::PadTemplate>> = Lazy::new(|| {
            let sink_pad_template = gst::ElementFactory::make("audioconvert")
                .build()
                .unwrap()
                .static_pad("sink")
                .unwrap()
                .pad_template()
                .unwrap();
            let audio_caps = sink_pad_template.caps();

            let video_caps = gst::Caps::new_any();

            vec![
                gst::PadTemplate::new(
                    "audio_sink",
                    gst::PadDirection::Sink,
                    gst::PadPresence::Request,
                    &audio_caps,
                )
                .unwrap(),
                gst::PadTemplate::new(
                    "video_sink",
                    gst::PadDirection::Sink,
                    gst::PadPresence::Request,
                    &video_caps,
                )
                .unwrap(),
                gst::PadTemplate::new(
                    "audio_src",
                    gst::PadDirection::Src,
                    gst::PadPresence::Sometimes,
                    &audio_caps,
                )
                .unwrap(),
                gst::PadTemplate::new(
                    "video_src",
                    gst::PadDirection::Src,
                    gst::PadPresence::Sometimes,
                    &video_caps,
                )
                .unwrap(),
            ]
        });

        PAD_TEMPLATES.as_ref()
    }

    fn request_new_pad(
        &self,
        _templ: &gst::PadTemplate,
        name: Option<&str>,
        _caps: Option<&gst::Caps>,
    ) -> Option<gst::Pad> {
        let name = name?;

        match name {
            "audio_sink" => {
                self.setup_audio_pipeline();
                let audio_sinkpad = self.audio().sinkpad.clone();

                Some(audio_sinkpad.upcast())
            }
            "video_sink" => {
                self.setup_video_pipeline();
                let video_sinkpad = self.video().sinkpad.clone();

                Some(video_sinkpad.upcast())
            }
            _ => None,
        }
    }

    fn change_state(
        &self,
        transition: gst::StateChange,
    ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
        gst::trace!(CAT, imp: self, "Changing state {:?}", transition);

        match transition {
            gst::StateChange::NullToReady => {
                self.prepare().map_err(|err| {
                    self.post_error_message(err);
                    gst::StateChangeError
                })?;
            }
            gst::StateChange::PlayingToPaused => {
                self.pause().map_err(|_| gst::StateChangeError)?;
            }
            gst::StateChange::ReadyToNull => {
                self.unprepare();
            }
            _ => (),
        }

        let success = self.parent_change_state(transition)?;

        match transition {
            gst::StateChange::PausedToPlaying => {
                self.play().map_err(|_| gst::StateChangeError)?;
            }
            gst::StateChange::PausedToReady => {
                self.stop().map_err(|_| gst::StateChangeError)?;
            }
            _ => (),
        }

        Ok(success)
    }
}

impl BinImpl for RendererBin {}
