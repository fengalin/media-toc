use glib::{glib_object_subclass, subclass::prelude::*};
use gst::{
    gst_debug, gst_error, gst_error_msg, gst_info, gst_trace, gst_warning, prelude::*,
    subclass::prelude::*, Element, Event, Pad,
};

use lazy_static::lazy_static;

use std::{
    convert::TryFrom,
    sync::{Mutex, MutexGuard},
};

use metadata::Duration;

use crate::{
    generic::{prelude::*, DoubleRenderer, GBoxedDoubleRendererImpl},
    plugin, Timestamp,
};

pub const NAME: &str = "mediatocrenderer";

pub const DBL_RENDERER_IMPL_PROP: &str = "dbl-renderer-impl";
pub const CLOCK_REF_PROP: &str = "clock-ref";
pub const BUFFER_SIZE_PROP: &str = "buffer-size";
pub const DEFAULT_BUFFER_SIZE: Duration = Duration::from_secs(5);

pub const MUST_REFRESH_SIGNAL: &str = "must-refresh";
pub const SEEK_DONE_SIGNAL: &str = "seek-done";

/// A two steps seek allows centering the audio representation around
/// target position in Paused mode. This helps the user visualize
/// the context around current position and interact with the
/// application.
///
/// A two steps seek is initiated when the [`Pipeline`](gst::Pipeline)
/// is in Paused mode and the position in the stream is appropriate.
/// The sequence unfolds as follows:
///
/// - A user seek is received and the conditions are appropriate for
///   a two steps seek.
/// - When the [`Segment`](gst::Segment) from the user seek is received,
///   a new seek is immediately sent upstream. This first step seek
///   aims at retrieving the samples required to render the audio
///   visualisation preceeding the user target position.
/// - Once the renderer has received enough samples, a second seek
///   is emitted in order to set the [`Pipeline`](gst::Pipeline) back to
///   the user requested position.
///
/// When the [`Renderer`] [`Element`](gst::Element) is used via a
/// [`RendererBin`], the first step seek can be filtered so that only
/// the rendering elements are affected. This allows reducing resources
/// usage.

/// Field indicating the [`RendererBin`] that this needs to be filtered.
///
/// When the [`RendererBin`] receives a seek [`Event`](gst::Event) containing
/// this field, it must send the events and buffers in the [`Segment`](gst::Segment)
/// with the same [`Seqnum`](gst::Seqnum) only to the rendering elements.
pub(crate) const FILTER_FIELD: &str = "filter";

lazy_static! {
    static ref CAT: gst::DebugCategory = gst::DebugCategory::new(
        NAME,
        gst::DebugColorFlags::empty(),
        Some("media-toc Renderer"),
    );
}

pub(crate) static PROPERTIES: [glib::subclass::Property; 3] = [
    glib::subclass::Property(DBL_RENDERER_IMPL_PROP, |name| {
        glib::ParamSpec::boxed(
            name,
            "Double Renderer",
            "Implementation for the Double Renderer",
            GBoxedDoubleRendererImpl::get_type(),
            glib::ParamFlags::READWRITE,
        )
    }),
    glib::subclass::Property(CLOCK_REF_PROP, |name| {
        glib::ParamSpec::object(
            name,
            "Clock reference",
            "Element providing the clock reference",
            gst::Element::static_type(),
            glib::ParamFlags::WRITABLE,
        )
    }),
    glib::subclass::Property(BUFFER_SIZE_PROP, |name| {
        glib::ParamSpec::uint64(
            name,
            "Renderer Size (ns)",
            "Internal buffer size in ns",
            1_000u64,
            u64::MAX,
            DEFAULT_BUFFER_SIZE.as_u64(),
            glib::ParamFlags::WRITABLE,
        )
    }),
];

#[derive(Debug)]
struct Settings {
    // FIXME use an enum to select the renderer and embed rendering
    // with the plugin
    dbl_renderer_impl: Option<Box<dyn DoubleRendererImpl>>,
    clock_ref: Option<gst::Element>,
    buffer_size: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dbl_renderer_impl: None,
            clock_ref: None,
            buffer_size: DEFAULT_BUFFER_SIZE,
        }
    }
}

#[derive(Debug)]
enum State {
    Started,
    Stopped,
    Paused,
    Prepared,
    Unprepared,
}

impl Default for State {
    fn default() -> Self {
        State::Unprepared
    }
}

impl State {
    fn is_started(&self) -> bool {
        matches!(self, State::Started)
    }
}

#[derive(Debug, Clone, Copy)]
enum SeekState {
    EarlySegment {
        seqnum: gst::Seqnum,
    },
    DoneOnFirstBuffer {
        target_ts: u64,
    },
    Step1 {
        step_2_ts: u64,
        buffer_filled_ts: u64,
    },
    PendingStep1Segment {
        seqnum: gst::Seqnum,
        step_2_ts: u64,
        buffer_filled_ts: u64,
    },
    PendingInitSeekSegment {
        seqnum: gst::Seqnum,
        step_1_ts: u64,
        step_2_ts: u64,
        buffer_filled_ts: u64,
    },
    PendingStep2Segment {
        seqnum: gst::Seqnum,
        buffer_filled_ts: u64,
    },
}

#[derive(Debug, Default)]
struct Context {
    dbl_renderer: Option<DoubleRenderer>,
    seek_state: Option<SeekState>,
    state: State,
    settings: Settings,
}

impl Context {
    #[inline]
    #[track_caller]
    fn dbl_renderer(&mut self) -> &mut DoubleRenderer {
        self.dbl_renderer.as_mut().expect("no dbl_renderer")
    }
}

pub struct Renderer {
    sinkpad: Pad,
    srcpad: Pad,
    ctx: Mutex<Context>,
}

/// Pads handler
impl Renderer {
    fn seek_done(
        mut ctx: MutexGuard<Context>,
        element: &plugin::Renderer,
        target_ts: impl Into<Timestamp>,
    ) {
        if ctx.seek_state.take().is_some() {
            ctx.dbl_renderer().seek_done(target_ts.into());
            drop(ctx);
            gst_debug!(CAT, obj: element, "seek done");
            element.emit(SEEK_DONE_SIGNAL, &[]).unwrap();
        }
    }

    fn cancel_pending_seek(mut ctx: MutexGuard<Context>, element: &plugin::Renderer) {
        if let Some(seek_state) = ctx.seek_state.take() {
            ctx.dbl_renderer().cancel_seek();
            drop(ctx);
            gst_info!(CAT, obj: element, "cancelling seek while {:?}", seek_state);
            // We might want to inform that the seek was cancelled
            element.emit(SEEK_DONE_SIGNAL, &[]).unwrap();
        }
    }

    fn sink_chain(
        &self,
        _pad: &Pad,
        element: &plugin::Renderer,
        buffer: gst::Buffer,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        use SeekState::*;

        let mut ctx = self.ctx.lock().unwrap();

        match ctx.seek_state {
            None | Some(EarlySegment { .. }) => {
                let must_notify =
                    ctx.dbl_renderer().push_gst_buffer(&buffer) && !ctx.state.is_started();
                drop(ctx);

                if must_notify {
                    element.emit(MUST_REFRESH_SIGNAL, &[]).unwrap();
                }
            }
            Some(DoneOnFirstBuffer { target_ts }) => {
                ctx.dbl_renderer().push_gst_buffer(&buffer);
                if let State::Paused = ctx.state {
                    ctx.dbl_renderer().refresh();
                }
                Self::seek_done(ctx, element, target_ts);

                gst_debug!(
                    CAT,
                    obj: element,
                    "seek with target {} done @ buffer {}",
                    buffer.get_pts(),
                    gst::ClockTime::from_nseconds(target_ts)
                );
            }
            Some(Step1 {
                step_2_ts,
                buffer_filled_ts,
            }) => {
                ctx.dbl_renderer().push_gst_buffer(&buffer);

                // Take a margin because the segment from the step 2 seek is always too late
                // FIXME find out why
                if buffer.get_pts().map_or(true, |pts| pts > buffer_filled_ts) {
                    // AudioBuffer filled passed the target buffer => seek to the actual target ts

                    let seek_event = gst::event::Seek::new(
                        1f64,
                        gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
                        gst::SeekType::Set,
                        step_2_ts.into(),
                        gst::SeekType::Set,
                        gst::ClockTime::none(),
                    );

                    let seqnum = seek_event.get_seqnum();

                    ctx.seek_state = Some(PendingStep2Segment {
                        seqnum,
                        buffer_filled_ts,
                    });
                    drop(ctx);

                    gst_debug!(
                        CAT,
                        obj: element,
                        "step 1 seek done @ {}, pushing step 2 {} {:?}",
                        buffer.get_pts(),
                        gst::ClockTime::from_nseconds(step_2_ts),
                        seqnum,
                    );

                    let sinkpad = self.sinkpad.clone();
                    element.call_async(move |_| {
                        sinkpad.push_event(seek_event);
                    });
                }
            }
            Some(PendingStep1Segment { .. })
            | Some(PendingInitSeekSegment { .. })
            | Some(PendingStep2Segment { .. }) => {
                // Wait for the new segment to be received on the src pad
            }
        }

        Ok(gst::FlowSuccess::Ok)
    }

    fn sink_event(&self, _pad: &Pad, element: &plugin::Renderer, event: Event) -> bool {
        use gst::EventView::*;

        fn unexpected(ctx: MutexGuard<Context>, element: &plugin::Renderer, event: &gst::Event) {
            gst_warning!(
                CAT,
                obj: element,
                "during seek {:?}, got {:?} with {:?}",
                ctx.seek_state,
                event.get_type(),
                event.get_seqnum(),
            );

            Renderer::cancel_pending_seek(ctx, element);
        }

        match event.view() {
            Caps(caps_evt) => {
                self.ctx
                    .lock()
                    .unwrap()
                    .dbl_renderer()
                    .set_caps(caps_evt.get_caps());
            }
            Eos(_) => {
                let mut ctx = self.ctx.lock().unwrap();
                // FIXME don't do that in case of a range playback
                ctx.dbl_renderer().handle_eos();

                if ctx.seek_state.take().is_some() {
                    drop(ctx);
                    element.emit(SEEK_DONE_SIGNAL, &[]).unwrap();
                }
            }
            FlushStart(_) => {
                let ctx = self.ctx.lock().unwrap();
                match ctx.seek_state {
                    None => (),
                    Some(SeekState::DoneOnFirstBuffer { .. }) => {
                        Self::cancel_pending_seek(ctx, element);
                    }
                    _ => return true,
                }
            }
            FlushStop(_) => {
                if self.ctx.lock().unwrap().seek_state.is_some() {
                    return true;
                }
            }
            Segment(segment_evt) => {
                let segment = segment_evt.get_segment();

                use SeekState::*;
                let mut ctx = self.ctx.lock().unwrap();
                match ctx.seek_state {
                    Some(PendingInitSeekSegment {
                        seqnum,
                        step_1_ts,
                        step_2_ts,
                        buffer_filled_ts,
                    }) => {
                        if seqnum == segment_evt.get_seqnum() {
                            if !self.srcpad.push_event(event) {
                                gst_error!(
                                    CAT,
                                    obj: element,
                                    "failed to push initial Segment {:?} downstream",
                                    seqnum,
                                );
                                ctx.seek_state = None;
                                return false;
                            }

                            return self.send_step1_seek(
                                ctx,
                                element,
                                step_1_ts,
                                step_2_ts,
                                buffer_filled_ts,
                            );
                        } else {
                            unexpected(ctx, &element, &event);
                        }
                    }
                    Some(PendingStep1Segment {
                        seqnum,
                        step_2_ts,
                        buffer_filled_ts,
                    }) => {
                        if seqnum == segment_evt.get_seqnum() {
                            gst_debug!(CAT, obj: element, "got step 1 seek segment {:?}", seqnum,);
                            ctx.dbl_renderer().have_gst_segment(segment);

                            ctx.seek_state = Some(Step1 {
                                step_2_ts,
                                buffer_filled_ts,
                            });

                            // Don't forward this segment donwstream
                            return true;
                        } else {
                            ctx.dbl_renderer().have_gst_segment(segment);
                            unexpected(ctx, &element, &event);
                        }
                    }
                    Some(PendingStep2Segment {
                        seqnum,
                        buffer_filled_ts,
                    }) => {
                        if seqnum == segment_evt.get_seqnum() {
                            gst_debug!(CAT, obj: element, "got step 2 seek segment {:?}", seqnum);
                            ctx.dbl_renderer().have_gst_segment(segment);

                            ctx.seek_state = Some(DoneOnFirstBuffer {
                                target_ts: buffer_filled_ts,
                            });

                            return true;
                        } else {
                            ctx.dbl_renderer().have_gst_segment(segment);
                            unexpected(ctx, &element, &event);
                        }
                    }
                    Some(DoneOnFirstBuffer { .. }) => {
                        ctx.dbl_renderer().have_gst_segment(segment);
                    }
                    None | Some(EarlySegment { .. }) | Some(Step1 { .. }) => {
                        ctx.seek_state = Some(EarlySegment {
                            seqnum: segment_evt.get_seqnum(),
                        });
                        ctx.dbl_renderer().have_gst_segment(segment);
                    }
                }
            }
            StreamStart(_) => {
                // FIXME isn't there a StreamChanged?
                // FIXME track the audio stream id in dbl_renderer
                /*
                let audio_has_changed =
                    info_rwlck.read().unwrap().streams.audio_changed;
                if audio_has_changed {
                    debug!("changing audio stream");
                    ctx.dbl_renderer().flush_start()
                    // FIXME purge the waveform queue
                    let dbl_renderer = &mut dbl_renderer.lock().unwrap();
                    // FIXME should be part of a flush stop
                    ctx.dbl_renderer().flush()
                    //dbl_renderer.clean_samples();
                }
                */
            }
            _ => (),
        }

        self.srcpad.push_event(event)
    }

    fn src_event(&self, pad: &Pad, element: &plugin::Renderer, event: Event) -> bool {
        if let gst::EventView::Seek(seek_evt) = event.view() {
            let initial_seqnum = seek_evt.get_seqnum();

            let mut ctx = self.ctx.lock().unwrap();

            ctx.dbl_renderer().seek_start();

            // FIXME issue on gstreamer-rs + applicable to other event_ref and possibly query_ref
            // for a more explicit return type for `get`
            let (_, _, _, start, _, stop) = seek_evt.get();
            let start = gst::ClockTime::try_from(start).ok();

            let target_ts = match start.and_then(|start| start.nseconds()) {
                Some(start) => start,
                None => {
                    // Not a Time seek or start is not defined, keep the event as is
                    Self::cancel_pending_seek(ctx, element);
                    gst_debug!(
                        CAT,
                        obj: element,
                        "not Time seek to {:?} {:?}",
                        start,
                        initial_seqnum,
                    );
                    return pad.event_default(Some(element), event);
                }
            };

            if let State::Started = ctx.state {
                // perform a regular seek
                ctx.seek_state = Some(SeekState::DoneOnFirstBuffer { target_ts });
                drop(ctx);
                gst_debug!(
                    CAT,
                    obj: element,
                    "regular seek to {} {:?}",
                    gst::ClockTime::from_nseconds(target_ts),
                    initial_seqnum,
                );
                return true;
            }

            // Paused

            if let Some(step_1_ts) = ctx.dbl_renderer().first_ts_for_two_steps_seek(target_ts) {
                // Take a margin otherwise the resulting segment is too late
                let mut buffer_filled_ts = target_ts + 50_000_000;

                if let Ok(stop) = gst::ClockTime::try_from(stop) {
                    // stop is known, don't seek further
                    if let Some(stop) = stop.nseconds() {
                        buffer_filled_ts = stop.min(buffer_filled_ts);
                    }
                }

                if let Some(SeekState::EarlySegment { seqnum }) = ctx.seek_state {
                    if initial_seqnum == seqnum {
                        // Already received the Segment for this seek
                        return self.send_step1_seek(
                            ctx,
                            element,
                            step_1_ts.as_u64(),
                            target_ts,
                            buffer_filled_ts,
                        );
                    }
                }

                gst_debug!(
                    CAT,
                    obj: element,
                    "scheduling 2 steps seek from {} to {} buffer filled @ {} after Segment {:?}",
                    gst::ClockTime::from_nseconds(step_1_ts.as_u64()),
                    gst::ClockTime::from_nseconds(target_ts),
                    gst::ClockTime::from_nseconds(buffer_filled_ts),
                    initial_seqnum,
                );

                ctx.seek_state = Some(SeekState::PendingInitSeekSegment {
                    seqnum: initial_seqnum,
                    step_1_ts: step_1_ts.as_u64(),
                    step_2_ts: target_ts,
                    buffer_filled_ts,
                });
            } else {
                // Just perform a regular seek
                gst_debug!(
                    CAT,
                    obj: element,
                    "regular seek to {} in Paused",
                    gst::ClockTime::from_nseconds(target_ts)
                );
                ctx.seek_state = Some(SeekState::DoneOnFirstBuffer { target_ts });
            }

            return true;
        }

        pad.event_default(Some(element), event)
    }

    fn send_step1_seek(
        &self,
        mut ctx: MutexGuard<Context>,
        element: &plugin::Renderer,
        step_1_ts: u64,
        step_2_ts: u64,
        buffer_filled_ts: u64,
    ) -> bool {
        let seek_event = gst::event::Seek::builder(
            1f64,
            gst::SeekFlags::ACCURATE | gst::SeekFlags::FLUSH,
            gst::SeekType::Set,
            step_1_ts.into(),
            gst::SeekType::Set,
            gst::ClockTime::none(),
        )
        .other_fields(&[(FILTER_FIELD, &step_2_ts)])
        .build();

        let seqnum = seek_event.get_seqnum();

        ctx.seek_state = Some(SeekState::PendingStep1Segment {
            seqnum,
            step_2_ts,
            buffer_filled_ts,
        });

        gst_debug!(
            CAT,
            obj: element,
            "pushing step 1 seek {} toward {} {:?}",
            gst::ClockTime::from_nseconds(step_1_ts),
            gst::ClockTime::from_nseconds(step_2_ts),
            seqnum,
        );

        drop(ctx);
        let ret = self.sinkpad.push_event(seek_event);
        if !ret {
            gst_error!(CAT, obj: element, "failed to push step 1 seek");
            self.ctx.lock().unwrap().seek_state = None;
        }

        ret
    }
}

/// Element handler
impl Renderer {
    fn prepare(&self, element: &plugin::Renderer) -> Result<(), gst::ErrorMessage> {
        let mut ctx = self.ctx.lock().unwrap();
        gst_debug!(CAT, obj: element, "Preparing");

        let dbl_renderer_impl = ctx.settings.dbl_renderer_impl.take().ok_or_else(|| {
            let msg = "Double Renderer implementation not set";
            gst_error!(CAT, "{}", &msg);
            gst_error_msg!(gst::CoreError::StateChange, ["{}", &msg])
        })?;

        // FIXME might just use parent plugin?
        let clock_ref = ctx.settings.clock_ref.as_ref().ok_or_else(|| {
            let msg = "Clock reference element not set";
            gst_error!(CAT, "{}", &msg);
            gst_error_msg!(gst::CoreError::StateChange, ["{}", &msg])
        })?;

        ctx.dbl_renderer = Some(DoubleRenderer::new(
            dbl_renderer_impl,
            ctx.settings.buffer_size,
            clock_ref,
        ));

        ctx.state = State::Prepared;
        gst_debug!(CAT, obj: element, "Prepared");
        Ok(())
    }

    fn unprepare(&self, element: &plugin::Renderer) {
        let mut ctx = self.ctx.lock().unwrap();
        gst_debug!(CAT, obj: element, "Unpreparing");

        let dbl_renderer_impl = ctx.dbl_renderer.take().map(DoubleRenderer::into_impl);
        assert!(dbl_renderer_impl.is_some());

        ctx.settings.dbl_renderer_impl = dbl_renderer_impl;

        ctx.state = State::Unprepared;
        gst_debug!(CAT, obj: element, "Unprepared");
    }

    fn stop(&self, element: &plugin::Renderer) -> Result<(), gst::ErrorMessage> {
        let mut ctx = self.ctx.lock().unwrap();
        gst_debug!(CAT, obj: element, "Stopping");

        ctx.state = State::Stopped;

        if ctx.seek_state.take().is_some() {
            element.emit(SEEK_DONE_SIGNAL, &[]).unwrap();
        }

        gst_debug!(CAT, obj: element, "Stopped");
        Ok(())
    }

    fn start(&self, element: &plugin::Renderer) -> Result<(), gst::ErrorMessage> {
        let mut ctx = self.ctx.lock().unwrap();
        gst_debug!(CAT, obj: element, "Starting");

        ctx.state = State::Started;
        gst_debug!(CAT, obj: element, "Started");
        Ok(())
    }

    fn pause(&self, element: &plugin::Renderer) -> Result<(), gst::ErrorMessage> {
        let mut ctx = self.ctx.lock().unwrap();
        gst_debug!(CAT, obj: element, "Pausing");

        ctx.dbl_renderer().refresh();

        if ctx.seek_state.take().is_some() {
            element.emit(SEEK_DONE_SIGNAL, &[]).unwrap();
        }

        ctx.state = State::Paused;
        gst_debug!(CAT, obj: element, "Paused");
        Ok(())
    }
}

/// Element init
impl Renderer {
    pub(crate) fn sink_pad_template() -> gst::PadTemplate {
        let sink_caps = gst::Caps::new_simple(
            "audio/x-raw",
            &[
                (
                    "format",
                    &gst::List::new(&[&gst_audio::AudioFormat::S16le.to_str()]),
                ),
                ("channels", &gst::IntRange::<i32>::new(1, 8)),
                ("layout", &"interleaved"),
            ],
        );

        gst::PadTemplate::new(
            "sink",
            gst::PadDirection::Sink,
            gst::PadPresence::Always,
            &sink_caps,
        )
        .unwrap()
    }

    pub(crate) fn src_pad_template() -> gst::PadTemplate {
        // FIXME actually define
        let src_caps =
            gst::Caps::new_simple("video/x-raw", &[("format", &gst::List::new(&[&"BGRA"]))]);
        gst::PadTemplate::new(
            "src",
            gst::PadDirection::Src,
            gst::PadPresence::Always,
            &src_caps,
        )
        .unwrap()
    }
}

impl ObjectSubclass for Renderer {
    const NAME: &'static str = "MediaTocRenderer";
    type Type = super::Renderer;
    type ParentType = Element;
    type Instance = gst::subclass::ElementInstanceStruct<Self>;
    type Class = glib::subclass::simple::ClassStruct<Self>;

    glib_object_subclass!();

    fn with_class(klass: &glib::subclass::simple::ClassStruct<Self>) -> Self {
        let templ = klass.get_pad_template("sink").unwrap();
        let sinkpad = Pad::builder_with_template(&templ, Some("sink"))
            .chain_function(|pad, parent, buffer| {
                Renderer::catch_panic_pad_function(
                    parent,
                    || Err(gst::FlowError::Error),
                    |this, element| this.sink_chain(pad, element, buffer),
                )
            })
            .event_function(|pad, parent, event| {
                Renderer::catch_panic_pad_function(
                    parent,
                    || false,
                    |this, element| this.sink_event(pad, element, event),
                )
            })
            .build();

        let templ = klass.get_pad_template("src").unwrap();
        let srcpad = Pad::builder_with_template(&templ, Some("src"))
            .event_function(|pad, parent, event| {
                Renderer::catch_panic_pad_function(
                    parent,
                    || false,
                    |this, element| this.src_event(pad, element, event),
                )
            })
            .build();

        Renderer {
            sinkpad,
            srcpad,
            ctx: Mutex::new(Context::default()),
        }
    }

    fn class_init(klass: &mut glib::subclass::simple::ClassStruct<Self>) {
        klass.set_metadata(
            "media-toc Audio Visualization Renderer",
            "Visualization",
            "Renders audio buffer so that the user can see samples before and after current position",
            "François Laignel <fengalin@free.fr>",
        );

        klass.add_pad_template(Self::sink_pad_template());
        klass.add_pad_template(Self::src_pad_template());

        // FIXME this one could be avoided with a dedicated widget
        klass.add_signal(
            MUST_REFRESH_SIGNAL,
            glib::SignalFlags::RUN_LAST,
            &[],
            glib::types::Type::Unit,
        );

        klass.add_signal(
            SEEK_DONE_SIGNAL,
            glib::SignalFlags::RUN_LAST,
            &[],
            glib::types::Type::Unit,
        );

        klass.install_properties(&PROPERTIES);
    }
}

impl ObjectImpl for Renderer {
    fn set_property(&self, _element: &plugin::Renderer, id: usize, value: &glib::Value) {
        let mut ctx = self.ctx.lock().unwrap();
        match PROPERTIES[id] {
            glib::subclass::Property(DBL_RENDERER_IMPL_PROP, ..) => {
                let gboxed = value
                    .get_some::<&GBoxedDoubleRendererImpl>()
                    .expect("type checked upstream");
                let dbl_renderer_impl: Option<Box<dyn DoubleRendererImpl>> = gboxed.into();
                // FIXME don't panic log an error
                if dbl_renderer_impl.is_none() {
                    panic!("dbl_renderer_impl already taken");
                }
                ctx.settings.dbl_renderer_impl = dbl_renderer_impl;
            }
            glib::subclass::Property(CLOCK_REF_PROP, ..) => {
                let clock_ref = value
                    .get::<gst::Element>()
                    .expect("type checked upstream")
                    // FIXME don't panic log an error
                    .expect("Value is None");
                ctx.settings.clock_ref = Some(clock_ref);
            }
            glib::subclass::Property(BUFFER_SIZE_PROP, ..) => {
                let buffer_size = value.get_some::<u64>().expect("type checked upstream");
                ctx.settings.buffer_size = Duration::from_nanos(buffer_size);
            }
            _ => unimplemented!(),
        }
    }

    fn get_property(&self, _element: &plugin::Renderer, id: usize) -> glib::Value {
        let mut ctx = self.ctx.lock().unwrap();
        match PROPERTIES[id] {
            glib::subclass::Property(DBL_RENDERER_IMPL_PROP, ..) => {
                if !matches!(ctx.state, State::Unprepared) {
                    panic!(
                        "retrieval of the dbl renderer impl in {:?} attempted",
                        ctx.state
                    );
                }

                let dbl_renderer_impl = ctx
                    .settings
                    .dbl_renderer_impl
                    .take()
                    // FIXME don't panic log an error
                    .expect("dbl renderer impl already taken");
                let gboxed: GBoxedDoubleRendererImpl = dbl_renderer_impl.into();
                gboxed.to_value()
            }
            _ => unimplemented!(),
        }
    }

    fn constructed(&self, element: &plugin::Renderer) {
        self.parent_constructed(element);

        element.add_pad(&self.sinkpad).unwrap();
        element.add_pad(&self.srcpad).unwrap();
    }
}

impl ElementImpl for Renderer {
    fn change_state(
        &self,
        element: &plugin::Renderer,
        transition: gst::StateChange,
    ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
        gst_trace!(CAT, obj: element, "Changing state {:?}", transition);

        match transition {
            gst::StateChange::NullToReady => {
                self.prepare(element).map_err(|err| {
                    element.post_error_message(err);
                    gst::StateChangeError
                })?;
            }
            gst::StateChange::PlayingToPaused => {
                self.pause(element).map_err(|_| gst::StateChangeError)?;
            }
            gst::StateChange::ReadyToNull => {
                self.unprepare(element);
            }
            _ => (),
        }

        let mut success = self.parent_change_state(element, transition)?;

        match transition {
            gst::StateChange::ReadyToPaused => {
                success = gst::StateChangeSuccess::NoPreroll;
            }
            gst::StateChange::PausedToPlaying => {
                self.start(element).map_err(|_| gst::StateChangeError)?;
            }
            gst::StateChange::PlayingToPaused => {
                success = gst::StateChangeSuccess::NoPreroll;
            }
            gst::StateChange::PausedToReady => {
                self.stop(element).map_err(|_| gst::StateChangeError)?;
            }
            _ => (),
        }

        Ok(success)
    }
}
