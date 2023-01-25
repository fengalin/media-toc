use gst::{
    self,
    glib::{self, subclass::Signal},
    prelude::*,
    subclass::prelude::*,
    ClockTime, Element, Event, Pad,
};

use once_cell::sync::Lazy;

use std::{convert::TryInto, sync::Mutex};

use metadata::Duration;

use crate::{
    generic::{prelude::*, DoubleRenderer, GBoxedDoubleRendererImpl, WindowTimestamps},
    plugin,
};

pub const NAME: &str = "mediatocrenderer";

// FIXME use an enum just like for SegmentField
pub const DBL_RENDERER_IMPL_PROP: &str = "dbl-renderer-impl";
pub const CLOCK_REF_PROP: &str = "clock-ref";
pub const BUFFER_SIZE_PROP: &str = "buffer-size";
pub const DEFAULT_BUFFER_SIZE: Duration = Duration::from_secs(5);

pub const GET_WINDOW_TIMESTAMPS_SIGNAL: &str = "get-window-ts";
pub const SEGMENT_DONE_SIGNAL: &str = "segment-done";
// FIXME remove when widget is handled locally
pub const MUST_REFRESH_SIGNAL: &str = "must-refresh";

static CAT: Lazy<gst::DebugCategory> = Lazy::new(|| {
    gst::DebugCategory::new(
        NAME,
        gst::DebugColorFlags::empty(),
        Some("media-toc Renderer"),
    )
});

// FIXME: merge Fields in a single enum?
pub enum SeekField {
    PlayRange,
}

impl SeekField {
    pub const fn as_str(&self) -> &str {
        "play-range"
    }
}

pub enum SegmentField {
    PlayRange,
    RestoringPosition,
    Stage1,
    Stage2,
}

impl SegmentField {
    pub const fn as_str(&self) -> &str {
        use SegmentField::*;
        match self {
            PlayRange => "play-range",
            RestoringPosition => "restore-ts",
            Stage1 => "stage-1",
            Stage2 => "stage-2",
        }
    }
}

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
    Paused,
    Playing,
    Prepared,
    Stopped,
    Unprepared,
}

impl Default for State {
    fn default() -> Self {
        State::Unprepared
    }
}

impl State {
    fn is_playing(&self) -> bool {
        matches!(self, State::Playing)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SeekState {
    Uncontrolled,
    DoneOn1stBuffer { target_ts: ClockTime },
    PlayRange,
    TwoStages,
}

impl SeekState {
    fn is_seeking(&self) -> bool {
        use SeekState::*;
        matches!(self, DoneOn1stBuffer { .. } | TwoStages)
    }

    fn take(&mut self) -> Self {
        std::mem::replace(self, SeekState::Uncontrolled)
    }
}

impl Default for SeekState {
    fn default() -> Self {
        SeekState::Uncontrolled
    }
}

#[derive(Debug, Default)]
struct Context {
    dbl_renderer: Option<DoubleRenderer>,
    seek: SeekState,
    segment_seqnum: Option<gst::Seqnum>,
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
    ctx: Mutex<Context>,
}

/// Sink Pad handler.
impl Renderer {
    fn sink_chain(
        &self,
        pad: &Pad,
        element: &plugin::Renderer,
        buffer: gst::Buffer,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        use SeekState::*;

        let mut ctx = self.ctx.lock().unwrap();
        //gst::trace!(CAT, obj: pad, "{:?} / {:?}", buffer, ctx.seek);
        match ctx.seek {
            Uncontrolled | PlayRange => {
                let must_notify =
                    ctx.dbl_renderer().push_buffer(&buffer) && !ctx.state.is_playing();
                drop(ctx);

                // FIXME push renderered buffer when we have a Visualization widget
                if must_notify {
                    element.emit_by_name::<()>(MUST_REFRESH_SIGNAL, &[]);
                }
            }
            TwoStages => {
                ctx.dbl_renderer().push_buffer(&buffer);
            }
            DoneOn1stBuffer { target_ts } => {
                ctx.dbl_renderer().push_buffer(&buffer);
                ctx.dbl_renderer().seek_done(target_ts.try_into().unwrap());
                if let State::Paused = ctx.state {
                    ctx.dbl_renderer().refresh();
                } else {
                    ctx.dbl_renderer().release();
                }
                ctx.seek = Uncontrolled;
                drop(ctx);

                gst::info!(CAT, obj: pad, "Streaming from {}", target_ts.display());

                element.emit_by_name::<()>(MUST_REFRESH_SIGNAL, &[]);
            }
        }

        Ok(gst::FlowSuccess::Ok)
    }

    fn sink_event(&self, pad: &Pad, element: &plugin::Renderer, event: Event) -> bool {
        use gst::EventView::*;
        use SeekState::*;

        match event.view() {
            Caps(caps_evt) => {
                self.ctx
                    .lock()
                    .unwrap()
                    .dbl_renderer()
                    .set_caps(caps_evt.caps());
            }
            Eos(_) => {
                let mut ctx = self.ctx.lock().unwrap();
                ctx.segment_seqnum = None;

                if ctx.seek.take().is_seeking() {
                    ctx.dbl_renderer().cancel_seek();
                    ctx.dbl_renderer().handle_eos();
                    drop(ctx);
                    gst::warning!(CAT, obj: pad, "reached EOS while seeking");
                } else {
                    ctx.dbl_renderer().handle_eos();
                    drop(ctx);
                    gst::debug!(CAT, obj: pad, "reached EOS");
                }

                return true;
            }
            Segment(evt) => {
                let mut ctx = self.ctx.lock().unwrap();
                let segment = evt.segment();
                let segment = match segment.downcast_ref::<gst::format::Time>() {
                    Some(segment) => segment,
                    None => {
                        // Not a Time segment, keep the event as is
                        drop(ctx);
                        gst::debug!(CAT, obj: pad, "not Time {:?} {:?}", segment, event.seqnum());
                        return gst::Pad::event_default(pad, Some(element), event);
                    }
                };

                let start = segment.time().unwrap_or(gst::ClockTime::ZERO);

                let mut was_handled = false;
                if let Some(structure) = evt.structure() {
                    if structure.has_field(SegmentField::Stage1.as_str()) {
                        gst::debug!(
                            CAT,
                            obj: pad,
                            "got stage 1 segment starting @ {} {:?}",
                            start.display(),
                            event.seqnum(),
                        );
                        ctx.dbl_renderer().seek_start();
                        ctx.dbl_renderer().have_segment(segment);
                        ctx.seek = TwoStages;
                        was_handled = true;
                    } else if structure.has_field(SegmentField::Stage2.as_str()) {
                        gst::debug!(
                            CAT,
                            obj: pad,
                            "got stage 2 segment starting @ {} {:?}",
                            start.display(),
                            event.seqnum(),
                        );
                        ctx.dbl_renderer().have_segment(segment);
                        ctx.seek = DoneOn1stBuffer { target_ts: start };
                        was_handled = true;
                    } else if structure.has_field(SegmentField::PlayRange.as_str()) {
                        gst::debug!(
                            CAT,
                            obj: pad,
                            "got play range segment starting @ {} {:?}",
                            start,
                            event.seqnum(),
                        );
                        ctx.dbl_renderer().have_segment(segment);
                        ctx.seek = PlayRange;
                        was_handled = true;
                    } else if structure.has_field(SegmentField::RestoringPosition.as_str()) {
                        gst::info!(
                            CAT,
                            obj: pad,
                            "got play range restoring segment starting @ {} {:?}",
                            start,
                            event.seqnum(),
                        );
                        ctx.dbl_renderer().have_segment(segment);
                        ctx.seek = DoneOn1stBuffer { target_ts: start };
                        was_handled = true;
                    }
                }

                if !was_handled {
                    gst::debug!(
                        CAT,
                        obj: pad,
                        "got segment starting @ {} {:?}",
                        start,
                        event.seqnum(),
                    );

                    if ctx.state.is_playing() {
                        ctx.dbl_renderer().release();
                    }
                    ctx.dbl_renderer().seek_start();
                    ctx.dbl_renderer().have_segment(segment);
                    ctx.seek = DoneOn1stBuffer { target_ts: start };
                }
            }
            SegmentDone(_) => {
                if self.ctx.lock().unwrap().seek == SeekState::TwoStages {
                    gst::debug!(CAT, obj: pad, "got segment done {:?}", event.seqnum());
                    element.emit_by_name::<()>(SEGMENT_DONE_SIGNAL, &[]);
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

        gst::Pad::event_default(pad, Some(element), event)
    }
}

#[glib::object_subclass]
impl ObjectSubclass for Renderer {
    const NAME: &'static str = "MediaTocRenderer";
    type Type = plugin::Renderer;
    type ParentType = Element;

    fn with_class(klass: &Self::Class) -> Self {
        let templ = klass.pad_template("sink").unwrap();
        let sinkpad = Pad::builder_with_template(&templ, Some("sink"))
            .chain_function(|pad, parent, buffer| {
                Renderer::catch_panic_pad_function(
                    parent,
                    || Err(gst::FlowError::Error),
                    |this| this.sink_chain(pad, &this.obj(), buffer),
                )
            })
            .event_function(|pad, parent, event| {
                Renderer::catch_panic_pad_function(
                    parent,
                    || false,
                    |this| this.sink_event(pad, &this.obj(), event),
                )
            })
            .build();

        Renderer {
            sinkpad,
            ctx: Mutex::new(Context::default()),
        }
    }
}

impl ObjectImpl for Renderer {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
            vec![
                glib::ParamSpecBoxed::new(
                    DBL_RENDERER_IMPL_PROP,
                    "Double Renderer",
                    "Implementation for the Double Renderer",
                    GBoxedDoubleRendererImpl::static_type(),
                    glib::ParamFlags::READWRITE,
                ),
                glib::ParamSpecObject::new(
                    CLOCK_REF_PROP,
                    "Clock reference",
                    "Element providing the clock reference",
                    gst::Element::static_type(),
                    glib::ParamFlags::WRITABLE,
                ),
                // FIXME use a ClockTime
                glib::ParamSpecUInt64::new(
                    BUFFER_SIZE_PROP,
                    "Renderer Size (ns)",
                    "Internal buffer size in ns",
                    1_000u64,
                    u64::MAX,
                    DEFAULT_BUFFER_SIZE.as_u64(),
                    glib::ParamFlags::WRITABLE,
                ),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn set_property(
        &self,
        _id: usize,
        value: &glib::Value,
        pspec: &glib::ParamSpec,
    ) {
        let mut ctx = self.ctx.lock().unwrap();
        match pspec.name() {
            DBL_RENDERER_IMPL_PROP => {
                let gboxed = value
                    .get::<GBoxedDoubleRendererImpl>()
                    .expect("type checked upstream");
                let dbl_renderer_impl: Option<Box<dyn DoubleRendererImpl>> = gboxed.into();
                // FIXME don't panic log an error
                if dbl_renderer_impl.is_none() {
                    panic!("dbl_renderer_impl already taken");
                }
                ctx.settings.dbl_renderer_impl = dbl_renderer_impl;
            }
            CLOCK_REF_PROP => {
                let clock_ref = value.get::<gst::Element>().expect("type checked upstream");
                ctx.settings.clock_ref = Some(clock_ref);
            }
            BUFFER_SIZE_PROP => {
                let buffer_size = value.get::<u64>().expect("type checked upstream");
                ctx.settings.buffer_size = Duration::from_nanos(buffer_size);
            }
            _ => unimplemented!(),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        let mut ctx = self.ctx.lock().unwrap();
        match pspec.name() {
            DBL_RENDERER_IMPL_PROP => {
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

    fn signals() -> &'static [Signal] {
        static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
            vec![
                Signal::builder(GET_WINDOW_TIMESTAMPS_SIGNAL)
                .return_type_from(WindowTimestamps::static_type())
                .run_last()
                .action()
                .class_handler(|_token, args| {
                    let element = args[0]
                        .get::<<Renderer as ObjectSubclass>::Type>()
                        .expect("Failed to get args[0]");

                    let window_ts = element
                        .imp()
                        .ctx
                        .lock()
                        .unwrap()
                        .dbl_renderer
                        .as_mut()
                        .and_then(|dbl_renderer| dbl_renderer.window_ts());
                    // FIXME also refresh rendering?
                    Some(window_ts.as_ref().to_value())
                })
                .build(),
                Signal::builder(SEGMENT_DONE_SIGNAL)
                    .run_last()
                    .build(),
                // FIXME this one could be avoided with a dedicated widget
                Signal::builder(MUST_REFRESH_SIGNAL)
                    .run_last()
                    .build(),
            ]
        });

        SIGNALS.as_ref()
    }

    fn constructed(&self) {
        self.parent_constructed();
        self.obj().add_pad(&self.sinkpad).unwrap();
    }
}

/// State change
impl Renderer {
    fn prepare(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp: self, "Preparing");
        let mut ctx = self.ctx.lock().unwrap();

        let dbl_renderer_impl = ctx.settings.dbl_renderer_impl.take().ok_or_else(|| {
            let msg = "Double Renderer implementation not set";
            gst::error!(CAT, "{}", &msg);
            gst::error_msg!(gst::CoreError::StateChange, ["{}", &msg])
        })?;

        let clock_ref = ctx.settings.clock_ref.as_ref().ok_or_else(|| {
            let msg = "Clock reference element not set";
            gst::error!(CAT, "{}", &msg);
            gst::error_msg!(gst::CoreError::StateChange, ["{}", &msg])
        })?;

        ctx.dbl_renderer = Some(DoubleRenderer::new(
            dbl_renderer_impl,
            ctx.settings.buffer_size,
            clock_ref,
        ));

        ctx.state = State::Prepared;
        gst::debug!(CAT, imp: self, "Prepared");
        Ok(())
    }

    fn play(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp: self, "Starting");
        let mut ctx = self.ctx.lock().unwrap();
        if !(ctx.seek == SeekState::PlayRange || ctx.seek.is_seeking()) {
            ctx.dbl_renderer().release();
        }
        ctx.state = State::Playing;
        gst::debug!(CAT, imp: self, "Started");
        Ok(())
    }

    fn pause(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp: self, "Pausing");
        let mut ctx = self.ctx.lock().unwrap();
        ctx.dbl_renderer().freeze();
        ctx.state = State::Paused;
        gst::debug!(CAT, imp: self, "Paused");
        Ok(())
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        gst::debug!(CAT, imp: self, "Stopping");
        let mut ctx = self.ctx.lock().unwrap();
        ctx.state = State::Stopped;
        ctx.seek = SeekState::Uncontrolled;
        gst::debug!(CAT, imp: self, "Stopped");
        Ok(())
    }

    fn unprepare(&self) {
        gst::debug!(CAT, imp: self, "Unpreparing");
        let mut ctx = self.ctx.lock().unwrap();
        let dbl_renderer_impl = ctx.dbl_renderer.take().map(DoubleRenderer::into_impl);
        assert!(dbl_renderer_impl.is_some());
        ctx.settings.dbl_renderer_impl = dbl_renderer_impl;
        ctx.state = State::Unprepared;
        gst::debug!(CAT, imp: self, "Unprepared");
    }
}

impl GstObjectImpl for Renderer {}

impl ElementImpl for Renderer {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: Lazy<gst::subclass::ElementMetadata> = Lazy::new(|| {
            gst::subclass::ElementMetadata::new(
                "media-toc Audio Visualization Renderer",
                "Visualization",
                "Renders audio buffer so that the user can see samples before and after current position",
                "François Laignel <fengalin@free.fr>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static PAD_TEMPLATES: Lazy<Vec<gst::PadTemplate>> = Lazy::new(|| {
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

            vec![gst::PadTemplate::new(
                "sink",
                gst::PadDirection::Sink,
                gst::PadPresence::Always,
                &sink_caps,
            )
            .unwrap()]
        });

        PAD_TEMPLATES.as_ref()
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
            gst::StateChange::ReadyToPaused => {
                self.pause().map_err(|_| gst::StateChangeError)?;
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
