use glib::{clone, glib_object_subclass, subclass::prelude::*};
use gst::{gst_debug, gst_trace, prelude::*, subclass::prelude::*};

use lazy_static::lazy_static;

use std::sync::{Arc, Mutex};

use crate::{generic::GBoxedDoubleRendererImpl, plugin};

pub const NAME: &str = "mediatocrendererbin";

lazy_static! {
    static ref CAT: gst::DebugCategory = gst::DebugCategory::new(
        NAME,
        gst::DebugColorFlags::empty(),
        Some("media-toc Renderer Bin"),
    );
}

static PROPERTIES: &[glib::subclass::Property; 3] = &plugin::renderer::PROPERTIES;

pub struct RendererBin {
    filter: Arc<Mutex<Option<gst::Seqnum>>>,
    audio_sinkpad: gst::GhostPad,
    renderer_srcpad: gst::GhostPad,
    audio_srcpad: gst::GhostPad,
    renderer: gst::Element,
    renderer_queue_sinkpad: gst::Pad,
    renderer_queue: gst::Element,
    audio_tee: gst::Element,
    audio_queue: gst::Element,
    video_sinkpad: gst::GhostPad,
    video_srcpad: gst::GhostPad,
    video_queue: gst::Element,
}

enum PadStream {
    Audio,
    Video,
}

/// Pad handler
impl RendererBin {
    fn sink_chain(
        &self,
        pad_stream: PadStream,
        pad: &gst::GhostPad,
        bin: &plugin::RendererBin,
        buffer: gst::Buffer,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        let filter = *self.filter.lock().unwrap();
        match filter {
            None => pad.chain_default(Some(bin), buffer),
            Some(_) => {
                if let PadStream::Audio = pad_stream {
                    // Forward to renderer queue only
                    self.renderer_queue_sinkpad.chain(buffer)
                } else {
                    Ok(gst::FlowSuccess::Ok)
                }
            }
        }
    }

    fn sink_event(
        &self,
        pad_stream: PadStream,
        pad: &gst::GhostPad,
        bin: &plugin::RendererBin,
        event: gst::Event,
    ) -> bool {
        // Note: when an unexpected event is detected, wait for the renderer
        // to decide whether to cancel the seek or not (by emitting SEEK_DONE_SIGNAL)

        use gst::EventView::*;
        match event.view() {
            FlushStart(_) => {
                let filter = *self.filter.lock().unwrap();
                if let Some(seqnum) = filter {
                    if seqnum == event.get_seqnum() {
                        gst_debug!(CAT, obj: pad, "filtering expected FlushStart {:?}", seqnum);

                        if let PadStream::Audio = pad_stream {
                            // forward to renderer elements only
                            return self.renderer_queue_sinkpad.send_event(event);
                        } else {
                            return true;
                        }
                    } else if let PadStream::Audio = pad_stream {
                        *self.filter.lock().unwrap() = None;
                        gst_debug!(
                            CAT,
                            obj: pad,
                            "got FlushStart {:?} cancelling filter {:?}",
                            event.get_seqnum(),
                            seqnum,
                        );
                    }
                }
            }
            FlushStop(_) => {
                let filter = *self.filter.lock().unwrap();
                if let Some(seqnum) = filter {
                    if seqnum == event.get_seqnum() {
                        gst_debug!(CAT, obj: pad, "filtering expected FlushStop {:?}", seqnum);

                        if let PadStream::Audio = pad_stream {
                            // forward to renderer elements only
                            return self.renderer_queue_sinkpad.send_event(event);
                        } else {
                            return true;
                        }
                    }
                }
            }
            Segment(_) => {
                let filter = *self.filter.lock().unwrap();
                if let Some(seqnum) = filter {
                    if seqnum == event.get_seqnum() {
                        gst_debug!(CAT, obj: pad, "filtering expected Segment {:?}", seqnum,);

                        if let PadStream::Audio = pad_stream {
                            // forward to renderer elements only
                            return self.renderer_queue_sinkpad.send_event(event);
                        } else {
                            return true;
                        }
                    }
                }
            }
            _ => (),
        }

        pad.event_default(Some(bin), event)
    }

    fn renderer_src_event(
        &self,
        pad: &gst::GhostPad,
        bin: &plugin::RendererBin,
        event: gst::Event,
    ) -> bool {
        if let gst::EventView::Seek(seek_evt) = event.view() {
            *self.filter.lock().unwrap() = None;
            gst_debug!(CAT, obj: bin, "app seek {:?}", seek_evt.get_seqnum());
        }

        pad.event_default(Some(bin), event)
    }

    fn renderer_sinkpad_probe(
        bin_audio_sinkpad: &gst::GhostPad,
        filter: &Arc<Mutex<Option<gst::Seqnum>>>,
        bin: &plugin::RendererBin,
        info: &mut gst::PadProbeInfo,
    ) -> gst::PadProbeReturn {
        if let Some(gst::PadProbeData::Event(event)) = info.data.as_ref() {
            if let gst::EventView::Seek(seek_evt) = event.view() {
                if seek_evt
                    .get_structure()
                    .iter()
                    .any(|structure| structure.has_field(plugin::FILTER_FIELD))
                {
                    let seqnum = seek_evt.get_seqnum();
                    *filter.lock().unwrap() = Some(seqnum);
                    gst_debug!(CAT, obj: bin, "detected seek to filter {:?}", seqnum);

                    // Forward upstream the bin
                    if let Some(gst::PadProbeData::Event(event)) = info.data.take() {
                        bin_audio_sinkpad.push_event(event);
                    } else {
                        unreachable!();
                    }

                    return gst::PadProbeReturn::Handled;
                }
            }
        }

        gst::PadProbeReturn::Ok
    }
}

/// Initialization
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

        // FIXME
        /*
        let first_buffer_printed = Arc::new(Mutex::new(false));
        self.renderer_queue_sinkpad.add_probe(
            gst::PadProbeType::BUFFER | gst::PadProbeType::EVENT_UPSTREAM | gst::PadProbeType::EVENT_DOWNSTREAM,
            move |_, info| {
                match info.data.as_ref() {
                    Some(gst::PadProbeData::Buffer(buffer)) => {
                        let mut first_buffer_printed = first_buffer_printed.lock().unwrap();
                        if !*first_buffer_printed {
                            println!("tee R src Buffer {:?}", buffer.get_pts());
                            *first_buffer_printed = true;
                        }
                    }
                    Some(gst::PadProbeData::Event(event)) => {
                        println!("tee R src {:?} {:?} {}",
                            event.get_type(),
                            event.get_seqnum(),
                            if event.is_downstream() { "downstream" } else { "upstream" },
                        );
                        *first_buffer_printed.lock().unwrap() = false;
                    }
                    _ => (),
                }
                gst::PadProbeReturn::Ok
            }
        );
        */

        let audio_tee_audio_src = self.audio_tee.get_request_pad("src_%u").unwrap();
        audio_tee_audio_src
            .link(&self.audio_queue.get_static_pad("sink").unwrap())
            .unwrap();

        // FIXME remove
        /*
        let first_buffer_printed = Arc::new(Mutex::new(false));
        audio_tee_audio_src.add_probe(
            gst::PadProbeType::BUFFER | gst::PadProbeType::EVENT_UPSTREAM | gst::PadProbeType::EVENT_DOWNSTREAM,
            move |_, info| {
                match info.data.as_ref() {
                    Some(gst::PadProbeData::Buffer(buffer)) => {
                        let mut first_buffer_printed = first_buffer_printed.lock().unwrap();
                        if !*first_buffer_printed {
                            println!("tee A src Buffer {:?}", buffer.get_pts());
                            *first_buffer_printed = true;
                        }
                    }
                    Some(gst::PadProbeData::Event(event)) => {
                        println!("tee A src {:?} {:?} {}",
                            event.get_type(),
                            event.get_seqnum(),
                            if event.is_downstream() { "downstream" } else { "upstream" },
                        );
                        *first_buffer_printed.lock().unwrap() = false;
                    }
                    _ => (),
                }
                gst::PadProbeReturn::Ok
            }
        );
        */

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
        self.renderer_queue.get_static_pad("sink").unwrap().add_probe(
            gst::PadProbeType::EVENT_UPSTREAM,
            clone!(@strong self.audio_sinkpad as bin_audio_sinkpad, @weak self.filter as filter, @strong bin => @default-panic, move |_, info| {
                Self::renderer_sinkpad_probe(&bin_audio_sinkpad, &filter, &bin, info)
            }),
        );

        self.renderer
            .connect(
                plugin::renderer::MUST_REFRESH_SIGNAL,
                true,
                clone!(@strong bin => move |_| {
                    bin
                        .emit(plugin::renderer::MUST_REFRESH_SIGNAL, &[])
                        .unwrap()
                }),
            )
            .unwrap();

        self.renderer
            .connect(
                plugin::renderer::SEEK_DONE_SIGNAL,
                true,
                clone!(@strong bin, @weak self.filter as filter => @default-panic, move |_| {
                    gst_debug!(CAT, obj: &bin, "forwarding seek done",);
                    *filter.lock().unwrap() = None;

                    bin
                        .emit(plugin::renderer::SEEK_DONE_SIGNAL, &[])
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
        let audio_sinkpad = gst::GhostPad::builder_with_template(
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

        let video_sinkpad = gst::GhostPad::builder_with_template(
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

        let renderer_srcpad = gst::GhostPad::builder_with_template(
            &klass.get_pad_template("renderer_src").unwrap(),
            Some("renderer_src"),
        )
        .event_function(|pad, parent, event| {
            RendererBin::catch_panic_pad_function(
                parent,
                || false,
                |this, element| this.renderer_src_event(pad, element, event),
            )
        })
        .build();

        let renderer_queue = Self::new_queue("renderer-queue");
        let renderer_queue_sinkpad = renderer_queue.get_static_pad("sink").unwrap();

        let audio_srcpad = gst::GhostPad::builder_with_template(
            &klass.get_pad_template("audio_src").unwrap(),
            Some("audio_src"),
        )
        .build();

        let video_srcpad = gst::GhostPad::builder_with_template(
            &klass.get_pad_template("video_src").unwrap(),
            Some("video_src"),
        )
        .build();

        RendererBin {
            filter: Arc::new(Mutex::new(None)),
            audio_sinkpad,
            renderer_srcpad,
            audio_srcpad,
            renderer: gst::ElementFactory::make(plugin::renderer::NAME, Some("media-toc-renderer"))
                .unwrap(),
            renderer_queue_sinkpad,
            renderer_queue,
            audio_tee: gst::ElementFactory::make("tee", Some("media-toc-renderer-audio-tee"))
                .unwrap(),
            audio_queue: Self::new_queue("audio-queue"),
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
            .get_caps()
            .unwrap();

        let video_caps = gst::Caps::new_any();

        let audio_sinkpad_tmpl = gst::PadTemplate::new(
            "audio_sink",
            gst::PadDirection::Sink,
            gst::PadPresence::Always,
            &audio_caps,
        )
        .unwrap();
        klass.add_pad_template(audio_sinkpad_tmpl);

        let renderer_caps = plugin::renderer::Renderer::src_pad_template()
            .get_caps()
            .unwrap();
        let renderer_srcpad_tmpl = gst::PadTemplate::new(
            "renderer_src",
            gst::PadDirection::Src,
            gst::PadPresence::Sometimes,
            &renderer_caps,
        )
        .unwrap();
        klass.add_pad_template(renderer_srcpad_tmpl);

        let video_sinkpad_tmpl = gst::PadTemplate::new(
            "video_sink",
            gst::PadDirection::Sink,
            gst::PadPresence::Always,
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

        klass.add_signal(
            plugin::renderer::SEEK_DONE_SIGNAL,
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

        self.renderer_srcpad
            .set_target(Some(&self.renderer.get_static_pad("src").unwrap()))
            .unwrap();
        bin.add_pad(&self.renderer_srcpad).unwrap();

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
        *self.filter.lock().unwrap() = None;
        gst_debug!(CAT, obj: bin, "Prepared");
        Ok(())
    }

    fn unprepare(&self, bin: &plugin::RendererBin) {
        gst_debug!(CAT, obj: bin, "Unpreparing");
        *self.filter.lock().unwrap() = None;
        gst_debug!(CAT, obj: bin, "Unprepared");
    }

    fn stop(&self, bin: &plugin::RendererBin) -> Result<(), gst::ErrorMessage> {
        gst_debug!(CAT, obj: bin, "Stopping");
        *self.filter.lock().unwrap() = None;
        gst_debug!(CAT, obj: bin, "Stopped");
        Ok(())
    }

    fn start(&self, bin: &plugin::RendererBin) -> Result<(), gst::ErrorMessage> {
        gst_debug!(CAT, obj: bin, "Starting");
        *self.filter.lock().unwrap() = None;
        gst_debug!(CAT, obj: bin, "Started");
        Ok(())
    }

    fn pause(&self, bin: &plugin::RendererBin) -> Result<(), gst::ErrorMessage> {
        gst_debug!(CAT, obj: bin, "Pausing");
        *self.filter.lock().unwrap() = None;
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

        let mut success = self.parent_change_state(bin, transition)?;

        match transition {
            gst::StateChange::ReadyToPaused => {
                success = gst::StateChangeSuccess::NoPreroll;
            }
            gst::StateChange::PausedToPlaying => {
                self.start(bin).map_err(|_| gst::StateChangeError)?;
            }
            gst::StateChange::PlayingToPaused => {
                success = gst::StateChangeSuccess::NoPreroll;
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
