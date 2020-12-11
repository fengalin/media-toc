use glib::{glib_object_subclass, subclass::prelude::*};
use gst::{
    gst_debug, gst_error, gst_error_msg, gst_info, gst_trace, gst_warning, prelude::*,
    subclass::prelude::*, ClockTime, Element, Event, Pad,
};

use lazy_static::lazy_static;

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

pub enum SegmentField {
    InitTwoStages,
    InWindow,
    Stage1,
    Stage2,
}

impl SegmentField {
    pub const fn as_str(&self) -> &str {
        use SegmentField::*;
        match self {
            InitTwoStages => "init-2-stages",
            InWindow => "in-window",
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

#[derive(Debug, Clone, Copy)]
enum SeekState {
    None,
    DoneOn1stBuffer { target_ts: ClockTime },
    TwoStages,
}

impl SeekState {
    fn is_seeking(&self) -> bool {
        use SeekState::*;
        matches!(self, DoneOn1stBuffer { .. } | TwoStages)
    }

    fn take(&mut self) -> Self {
        std::mem::replace(self, SeekState::None)
    }
}

impl Default for SeekState {
    fn default() -> Self {
        SeekState::None
    }
}

#[derive(Debug, Default)]
struct Context {
    dbl_renderer: Option<DoubleRenderer>,
    seek: SeekState,
    segment_seqnum: Option<gst::Seqnum>,
    in_window_segment: bool,
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
        gst_trace!(CAT, obj: pad, "{:?} / {:?}", buffer, ctx.seek);
        match ctx.seek {
            None => {
                let must_notify =
                    ctx.dbl_renderer().push_gst_buffer(&buffer) && !ctx.state.is_playing();
                drop(ctx);

                // FIXME push renderered buffer when we have a Visualization widget
                if must_notify {
                    element.emit(MUST_REFRESH_SIGNAL, &[]).unwrap();
                }
            }
            TwoStages => {
                ctx.dbl_renderer().push_gst_buffer(&buffer);
            }
            DoneOn1stBuffer { target_ts } => {
                ctx.dbl_renderer().push_gst_buffer(&buffer);
                if let State::Paused = ctx.state {
                    ctx.dbl_renderer().refresh();
                }
                ctx.dbl_renderer().seek_done(target_ts.try_into().unwrap());
                ctx.seek = None;
                drop(ctx);

                gst_info!(
                    CAT,
                    obj: pad,
                    "seek with target {} done @ buffer {}",
                    target_ts,
                    buffer.get_pts(),
                );

                element.emit(MUST_REFRESH_SIGNAL, &[]).unwrap();
            }
        }

        Ok(gst::FlowSuccess::Ok)
    }

    fn sink_event(&self, pad: &Pad, element: &plugin::Renderer, event: Event) -> bool {
        use gst::EventView::*;

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
                ctx.segment_seqnum = None;

                if ctx.seek.take().is_seeking() {
                    ctx.dbl_renderer().cancel_seek();
                    ctx.dbl_renderer().handle_eos();
                    drop(ctx);
                    gst_warning!(CAT, obj: pad, "reached EOS while seeking");
                } else {
                    ctx.dbl_renderer().handle_eos();
                    drop(ctx);
                    gst_debug!(CAT, obj: pad, "reached EOS");
                    //element.emit(MUST_REFRESH_SIGNAL, &[]).unwrap();
                }

                // Don't foward yet, we still have frames to render.
                return true;
            }
            Segment(evt) => {
                let mut ctx = self.ctx.lock().unwrap();
                let segment = evt.get_segment();
                let segment = match segment.downcast_ref::<gst::format::Time>() {
                    Some(segment) => segment,
                    None => {
                        // Not a Time segment, keep the event as is
                        drop(ctx);
                        gst_debug!(
                            CAT,
                            obj: pad,
                            "not Time {:?} {:?}",
                            segment,
                            event.get_seqnum()
                        );
                        return pad.event_default(Some(element), event);
                    }
                };

                // FIXME we used to replace None with zero here, can't remember the edge case
                let start = segment.get_time();

                let mut was_handled = false;
                if let Some(structure) = evt.get_structure() {
                    if structure.has_field(SegmentField::InitTwoStages.as_str()) {
                        gst_info!(
                            CAT,
                            obj: pad,
                            "starting 2 stages seek to {} {:?}",
                            start,
                            event.get_seqnum(),
                        );
                        ctx.dbl_renderer().seek_start();
                        ctx.seek = SeekState::TwoStages;
                        was_handled = true;
                    } else if structure.has_field(SegmentField::Stage1.as_str()) {
                        gst_debug!(
                            CAT,
                            obj: pad,
                            "got stage 1 segment starting @ {} {:?}",
                            start,
                            event.get_seqnum(),
                        );
                        ctx.dbl_renderer().have_gst_segment(segment);
                        was_handled = true;
                    } else if structure.has_field(SegmentField::Stage2.as_str()) {
                        gst_debug!(
                            CAT,
                            obj: pad,
                            "got stage 2 segment starting @ {} {:?}",
                            start,
                            event.get_seqnum(),
                        );
                        ctx.dbl_renderer().have_gst_segment(segment);
                        ctx.seek = SeekState::DoneOn1stBuffer { target_ts: start };
                        was_handled = true;
                    } else if structure.has_field(SegmentField::InWindow.as_str()) {
                        gst_info!(
                            CAT,
                            obj: pad,
                            "got segment in window starting @ {} {:?}",
                            start,
                            event.get_seqnum(),
                        );
                        ctx.dbl_renderer().freeze();
                        ctx.dbl_renderer().seek_start();
                        ctx.dbl_renderer().have_gst_segment(segment);
                        ctx.seek = SeekState::DoneOn1stBuffer { target_ts: start };
                        was_handled = true;
                    }
                }

                if !was_handled {
                    gst_info!(
                        CAT,
                        obj: pad,
                        "got segment starting @ {} {:?}",
                        start,
                        event.get_seqnum(),
                    );
                    if ctx.state.is_playing() {
                        ctx.dbl_renderer().release();
                    }
                    ctx.dbl_renderer().seek_start();
                    ctx.dbl_renderer().have_gst_segment(segment);
                    ctx.seek = SeekState::DoneOn1stBuffer { target_ts: start };
                }
            }
            SegmentDone(_) => {
                gst_debug!(CAT, obj: pad, "got segment done {:?}", event.get_seqnum(),);
                element.emit(SEGMENT_DONE_SIGNAL, &[]).unwrap();
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

        pad.event_default(Some(element), event)
    }
}

/// Element init
impl Renderer {
    pub fn sink_pad_template() -> gst::PadTemplate {
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

        Renderer {
            sinkpad,
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

        klass.add_signal_with_class_handler(
            GET_WINDOW_TIMESTAMPS_SIGNAL,
            glib::SignalFlags::RUN_LAST | glib::SignalFlags::ACTION,
            &[],
            WindowTimestamps::static_type(),
            |_, args| {
                let obj = args[0]
                    .get::<Self::Type>()
                    .expect("Failed to get args[0]")
                    .expect("Failed to get Object from args[0]");

                let this = Self::from_instance(&obj);
                let window_ts = this
                    .ctx
                    .lock()
                    .unwrap()
                    .dbl_renderer
                    .as_mut()
                    .and_then(|dbl_renderer| dbl_renderer.window_ts());
                // FIXME also refresh rendering?
                window_ts.as_ref().map(ToValue::to_value)
            },
        );

        klass.add_signal(
            SEGMENT_DONE_SIGNAL,
            glib::SignalFlags::RUN_LAST,
            &[],
            glib::types::Type::Unit,
        );

        // FIXME this one could be avoided with a dedicated widget
        klass.add_signal(
            MUST_REFRESH_SIGNAL,
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
    }
}

/// State change
impl Renderer {
    fn prepare(&self, element: &plugin::Renderer) -> Result<(), gst::ErrorMessage> {
        let mut ctx = self.ctx.lock().unwrap();
        gst_debug!(CAT, obj: element, "Preparing");

        let dbl_renderer_impl = ctx.settings.dbl_renderer_impl.take().ok_or_else(|| {
            let msg = "Double Renderer implementation not set";
            gst_error!(CAT, "{}", &msg);
            gst_error_msg!(gst::CoreError::StateChange, ["{}", &msg])
        })?;

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

    fn play(&self, element: &plugin::Renderer) -> Result<(), gst::ErrorMessage> {
        let mut ctx = self.ctx.lock().unwrap();

        if !(ctx.in_window_segment || ctx.seek.is_seeking()) {
            ctx.dbl_renderer().release();
        }
        ctx.state = State::Playing;

        gst_debug!(CAT, obj: element, "Playing");
        Ok(())
    }

    fn pause(&self, element: &plugin::Renderer) -> Result<(), gst::ErrorMessage> {
        let mut ctx = self.ctx.lock().unwrap();
        gst_debug!(CAT, obj: element, "Pausing");

        ctx.dbl_renderer().freeze();
        ctx.state = State::Paused;

        gst_debug!(CAT, obj: element, "Paused");
        Ok(())
    }

    fn stop(&self, element: &plugin::Renderer) -> Result<(), gst::ErrorMessage> {
        let mut ctx = self.ctx.lock().unwrap();
        gst_debug!(CAT, obj: element, "Stopping");

        ctx.state = State::Stopped;

        gst_debug!(CAT, obj: element, "Stopped");
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
            gst::StateChange::ReadyToPaused => {
                self.pause(element).map_err(|_| gst::StateChangeError)?;
            }
            _ => (),
        }

        let success = self.parent_change_state(element, transition)?;

        match transition {
            gst::StateChange::PausedToPlaying => {
                self.play(element).map_err(|_| gst::StateChangeError)?;
            }
            gst::StateChange::PausedToReady => {
                self.stop(element).map_err(|_| gst::StateChangeError)?;
            }
            _ => (),
        }

        Ok(success)
    }
}
