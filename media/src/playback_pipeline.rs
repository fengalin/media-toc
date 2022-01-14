use futures::{
    channel::{mpsc as async_mpsc, oneshot},
    prelude::*,
};

use gettextrs::gettext;
use glib::{clone, prelude::*, Cast, ObjectExt};
use gst::{prelude::*, ClockTime};

use log::warn;

use std::{
    borrow::Borrow,
    collections::HashSet,
    convert::AsRef,
    fmt,
    path::Path,
    sync::{Arc, RwLock},
};

use crate::{MediaEvent, QUEUE_SIZE};
use metadata::{media_info, Duration, MediaInfo};
use renderers::{
    generic::{self, prelude::*},
    plugin, Timestamp,
};

const RENDERER_BIN_NAME: &str = "media-toc-renderer-bin";

pub struct MissingPlugins(HashSet<String>);

impl MissingPlugins {
    fn new() -> Self {
        MissingPlugins(HashSet::<String>::new())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    fn insert(&mut self, plugin: String) {
        self.0.insert(plugin);
    }
}

impl fmt::Debug for MissingPlugins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner_res = fmt::Display::fmt(&self, f);
        f.debug_tuple("MissingPlugins").field(&inner_res).finish()
    }
}

impl fmt::Display for MissingPlugins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (idx, plugin) in self.0.iter().enumerate() {
            if idx > 0 {
                f.write_str("\n")?;
            }
            f.write_str("- ")?;
            f.write_str(plugin)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum OpenError {
    GLSinkError,
    Generic(String),
    MissingPlugins(MissingPlugins),
    StateChange,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use OpenError::*;

        match self {
            GLSinkError => write!(f, "Media: error with GL Sink"),
            Generic(err) => write!(f, "Media: error opening media {}", err),
            MissingPlugins(missing) => write!(f, "Media: found missing plugins {}", missing),
            StateChange => write!(f, "Media: state change error opening media"),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<gst::StateChangeError> for OpenError {
    fn from(_: gst::StateChangeError) -> Self {
        OpenError::StateChange
    }
}

#[derive(Debug)]
struct PurgeError;

#[derive(Debug)]
pub struct StateChangeError;

impl From<gst::StateChangeError> for StateChangeError {
    fn from(_: gst::StateChangeError) -> Self {
        StateChangeError
    }
}

impl From<PurgeError> for StateChangeError {
    fn from(_: PurgeError) -> Self {
        StateChangeError
    }
}

impl fmt::Display for StateChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Media: couldn't change state")
    }
}
impl std::error::Error for StateChangeError {}

#[derive(Debug)]
pub enum SeekError {
    Eos,
    Unrecoverable,
}

impl From<PurgeError> for SeekError {
    fn from(_: PurgeError) -> Self {
        SeekError::Unrecoverable
    }
}

impl fmt::Display for SeekError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use SeekError::*;

        match self {
            Eos => write!(f, "Media: seeking past the end"),
            Unrecoverable => write!(f, "Media: couldn't seek"),
        }
    }
}
impl std::error::Error for SeekError {}

#[derive(Debug)]
pub enum SelectStreamsError {
    UnknownId(Arc<str>),
    Unrecoverable,
}

impl From<media_info::SelectStreamError> for SelectStreamsError {
    fn from(err: media_info::SelectStreamError) -> Self {
        SelectStreamsError::UnknownId(Arc::clone(err.id()))
    }
}

impl From<PurgeError> for SelectStreamsError {
    fn from(_: PurgeError) -> Self {
        SelectStreamsError::Unrecoverable
    }
}

impl fmt::Display for SelectStreamsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectStreamsError::UnknownId(id) => {
                write!(f, "Media: select stream: unknown id {}", id.as_ref())
            }
            SelectStreamsError::Unrecoverable => write!(f, "Media: couldn't select stream"),
        }
    }
}
impl std::error::Error for SelectStreamsError {}

pub struct PlaybackPipeline {
    pipeline: gst::Pipeline,
    renderer: gst::Element,

    pub info: Arc<RwLock<MediaInfo>>,
    pub missing_plugins: MissingPlugins,
    int_evt_rx: async_mpsc::UnboundedReceiver<MediaEvent>,
    bus_watch_src_id: Option<glib::SourceId>,
}

/// Initialization
impl PlaybackPipeline {
    pub async fn try_new(
        path: &Path,
        dbl_visu_renderer: Box<dyn DoubleRendererImpl>,
        video_sink: &Option<gst::Element>,
    ) -> Result<(PlaybackPipeline, async_mpsc::UnboundedReceiver<MediaEvent>), OpenError> {
        plugin::init();

        // FIXME once we get rid of DBL_RENDERER_IMPL_PROP,
        // we should be able to intantiate the renderer while building the pipeline
        // and avoid keeping it as a field.
        let renderer =
            gst::ElementFactory::make(plugin::RENDERER_BIN_NAME, Some(RENDERER_BIN_NAME)).unwrap();
        renderer
            .set_property(
                plugin::DBL_RENDERER_IMPL_PROP,
                &generic::GBoxedDoubleRendererImpl::from(dbl_visu_renderer),
            )
            .unwrap();
        renderer
            .set_property(plugin::BUFFER_SIZE_PROP, &QUEUE_SIZE.as_u64())
            .unwrap();

        let (ext_evt_tx, ext_evt_rx) = async_mpsc::unbounded();
        let (int_evt_tx, int_evt_rx) = async_mpsc::unbounded();

        // FIXME remove in favor of a dedicate Visualization widget
        renderer
            .connect(
                plugin::MUST_REFRESH_SIGNAL,
                false,
                clone!(@strong ext_evt_tx => move |_| {
                    ext_evt_tx.unbounded_send(MediaEvent::MustRefresh).unwrap();
                    None
                }),
            )
            .unwrap();

        let mut this = PlaybackPipeline {
            pipeline: gst::Pipeline::new(Some("playback_pipeline")),
            renderer,
            // FIXME use pbutils
            info: Arc::new(RwLock::new(MediaInfo::new(path))),
            // FIXME use pbutils
            missing_plugins: MissingPlugins::new(),
            int_evt_rx,
            bus_watch_src_id: None,
        };

        this.build_pipeline(path, video_sink);

        let this = Self::open(this, int_evt_tx, ext_evt_tx).await?;
        Ok((this, ext_evt_rx))
    }

    pub fn check_requirements() -> Result<(), String> {
        gst::ElementFactory::make("decodebin3", None)
            .map(drop)
            .map_err(|_| gettext("Missing `decodebin3`\ncheck your gst-plugins-base install"))?;
        gst::ElementFactory::make("gtksink", None)
            .map(drop)
            .map_err(|_| {
                let (major, minor, _micro, _nano) = gst::version();
                let (variant1, variant2) = if major >= 1 && minor >= 14 {
                    ("gstreamer1-plugins-base", "gstreamer1.0-plugins-base")
                } else {
                    (
                        "gstreamer1-plugins-bad-free-gtk",
                        "gstreamer1.0-plugins-bad",
                    )
                };
                format!(
                    "{} {}\n{}",
                    gettext("Couldn't find GStreamer GTK video sink."),
                    gettext("Video playback will be disabled."),
                    gettext("Please install {} or {}, depending on your distribution.")
                        .replacen("{}", variant1, 1)
                        .replacen("{}", variant2, 1),
                )
            })
    }

    pub fn take_dbl_renderer_impl(&self) -> Box<dyn DoubleRendererImpl> {
        let dbl_visu_renderer_impl: Option<Box<dyn DoubleRendererImpl>> = self
            .renderer
            .get_property(plugin::DBL_RENDERER_IMPL_PROP)
            .expect("Prop double visu renderer impl not found")
            .get_some::<&generic::GBoxedDoubleRendererImpl>()
            .unwrap()
            .into();
        dbl_visu_renderer_impl.expect("double visu renderer impl already taken")
    }

    fn setup_queue(queue: &gst::Element) {
        queue.set_property("max-size-bytes", &0u32).unwrap();
        queue.set_property("max-size-buffers", &0u32).unwrap();
        queue
            .set_property("max-size-time", &QUEUE_SIZE.as_u64())
            .unwrap();

        #[cfg(feature = "trace-playback-queues")]
        queue
            .connect("overrun", false, |args| {
                let queue = args[0].get::<gst::Element>().unwrap().unwrap();
                warn!(
                    "OVERRUN {} (max-sizes: bytes {:?}, buffers {:?}, time {:?})",
                    queue.get_name(),
                    queue
                        .get_property("max-size-bytes")
                        .unwrap()
                        .get_some::<u32>()
                        .unwrap(),
                    queue
                        .get_property("max-size-buffers")
                        .unwrap()
                        .get_some::<u32>()
                        .unwrap(),
                    queue
                        .get_property("max-size-time")
                        .unwrap()
                        .get_some::<u64>()
                        .unwrap(),
                );
                None
            })
            .ok()
            .unwrap();
        #[cfg(feature = "trace-playback-queues")]
        queue
            .connect("underrun", false, |args| {
                let queue = args[0].get::<gst::Element>().unwrap().unwrap();
                warn!(
                    "UNDERRUN {} (max-sizes: bytes {:?}, buffers {:?}, time {:?})",
                    queue.get_name(),
                    queue
                        .get_property("max-size-bytes")
                        .unwrap()
                        .get_some::<u32>()
                        .unwrap(),
                    queue
                        .get_property("max-size-buffers")
                        .unwrap()
                        .get_some::<u32>()
                        .unwrap(),
                    queue
                        .get_property("max-size-time")
                        .unwrap()
                        .get_some::<u64>()
                        .unwrap(),
                );
                None
            })
            .ok()
            .unwrap();
    }

    fn build_pipeline(&mut self, path: &Path, video_sink: &Option<gst::Element>) {
        let decodebin = gst::ElementFactory::make("decodebin3", Some("decodebin")).unwrap();
        self.pipeline.add(&decodebin).unwrap();
        // From decodebin3's documentation: "Children: multiqueue0"
        let decodebin_as_bin = decodebin.clone().downcast::<gst::Bin>().ok().unwrap();
        let decodebin_multiqueue = &decodebin_as_bin.get_children()[0];
        PlaybackPipeline::setup_queue(decodebin_multiqueue);
        // Discard "interleave" as it modifies "max-size-time"
        decodebin_multiqueue
            .set_property("use-interleave", &false)
            .unwrap();

        let file_src = gst::ElementFactory::make("filesrc", Some("filesrc")).unwrap();
        file_src
            .set_property("location", &path.to_str().unwrap())
            .unwrap();
        self.pipeline.add(&file_src).unwrap();
        file_src.link(&decodebin).unwrap();

        let audio_sink =
            gst::ElementFactory::make("autoaudiosink", Some("audio-playback-sink")).unwrap();

        // Prepare pad configuration callback
        let pipeline_clone = self.pipeline.clone();
        let video_sink = video_sink.clone();
        let renderer = self.renderer.clone();

        decodebin.connect_pad_added(move |_decodebin, src_pad| {
            let pipeline = &pipeline_clone;
            let name = src_pad.get_name();

            if name.starts_with("audio_") {
                PlaybackPipeline::build_audio_pipeline(pipeline, src_pad, &audio_sink, &renderer);
            } else if name.starts_with("video_") {
                if let Some(ref video_sink) = video_sink {
                    PlaybackPipeline::build_video_pipeline(
                        pipeline, src_pad, video_sink, &renderer,
                    );
                }
            } else {
                // TODO: handle subtitles
                /*let fakesink = gst::ElementFactory::make("fakesink", None).unwrap();
                pipeline.add(&fakesink).unwrap();
                let fakesink_sink_pad = fakesink.get_static_pad("sink").unwrap();
                assert_eq!(src_pad.link(&fakesink_sink_pad), gst::PadLinkReturn::Ok);
                fakesink.sync_state_with_parent().unwrap();*/
            }
        });
    }

    fn build_audio_pipeline(
        pipeline: &gst::Pipeline,
        src_pad: &gst::Pad,
        audio_sink: &gst::Element,
        renderer: &gst::Element,
    ) {
        if pipeline.get_by_name(RENDERER_BIN_NAME).is_none() {
            pipeline.add(renderer).unwrap();
            // FIXME move out when the bin's internal pipelines are constructed on demand
            renderer
                .set_property(plugin::CLOCK_REF_PROP, audio_sink)
                .unwrap();
        }

        let audio_convert =
            gst::ElementFactory::make("audioconvert", Some("audio-audioconvert")).unwrap();
        let audio_resample =
            gst::ElementFactory::make("audioresample", Some("audio-audioresample")).unwrap();

        let elements = &[&audio_convert, &audio_resample, audio_sink];
        pipeline.add_many(elements).unwrap();

        src_pad
            .link(&renderer.get_request_pad("audio_sink").unwrap())
            .unwrap();

        renderer
            .link_pads(Some("audio_src"), &audio_convert, Some("sink"))
            .unwrap();

        gst::Element::link_many(&[&audio_convert, &audio_resample, audio_sink]).unwrap();

        renderer.sync_state_with_parent().unwrap();
        for e in elements {
            e.sync_state_with_parent().unwrap();
        }
    }

    fn build_video_pipeline(
        pipeline: &gst::Pipeline,
        src_pad: &gst::Pad,
        video_sink: &gst::Element,
        renderer: &gst::Element,
    ) {
        if pipeline.get_by_name(RENDERER_BIN_NAME).is_none() {
            pipeline.add(renderer).unwrap();
            // FIXME remove when the bin's internal pipelines are constructed on demand
            println!("CLOCK_REF video_sink");
            renderer
                .set_property(plugin::CLOCK_REF_PROP, video_sink)
                .unwrap();
        }

        src_pad
            .link(&renderer.get_request_pad("video_sink").unwrap())
            .unwrap();

        let convert = gst::ElementFactory::make("videoconvert", None).unwrap();
        let scale = gst::ElementFactory::make("videoscale", None).unwrap();

        let elements = &[&convert, &scale, video_sink];
        pipeline.add_many(elements).unwrap();

        renderer
            .link_pads(Some("video_src"), &convert, Some("sink"))
            .unwrap();

        gst::Element::link_many(elements).unwrap();

        renderer.sync_state_with_parent().unwrap();
        for e in elements {
            // Silently ignore the state sync issues
            // and rely on the PlaybackPipeline state to return an error.
            // Can't use sender to return the error because
            // the bus catches it first.
            let _res = e.sync_state_with_parent();
        }
    }

    async fn open(
        mut self,
        int_evt_tx: async_mpsc::UnboundedSender<MediaEvent>,
        ext_evt_tx: async_mpsc::UnboundedSender<MediaEvent>,
    ) -> Result<Self, OpenError> {
        let pipeline = self.pipeline.clone();

        let (handler_res_tx, handler_res_rx) = oneshot::channel();
        Self::register_open_bus_watch(self, handler_res_tx);

        pipeline.set_state(gst::State::Paused)?;
        self = handler_res_rx.await.unwrap()?;

        self.register_operations_bus_watch(int_evt_tx, ext_evt_tx);

        Ok(self)
    }

    fn register_open_bus_watch(self, handler_res_tx: oneshot::Sender<Result<Self, OpenError>>) {
        let mut handler_res_tx = Some(handler_res_tx);
        let pipeline = self.pipeline.clone();
        let mut this = Some(self);

        let mut streams_selected = false;

        pipeline
            .get_bus()
            .unwrap()
            .add_watch(move |_, msg| {
                use gst::MessageView::*;

                match msg.view() {
                    Error(err) => {
                        let mut this = this.take().unwrap();
                        this.cleanup();

                        if "sink" == err.get_src().unwrap().get_name() {
                            // Failure detected on a sink, this occurs when the GL sink
                            // can't operate properly
                            let _ = handler_res_tx
                                .take()
                                .unwrap()
                                .send(Err(OpenError::GLSinkError));

                            return glib::Continue(false);
                        }

                        let PlaybackPipeline {
                            missing_plugins, ..
                        } = this;
                        if !missing_plugins.is_empty() {
                            let _ = handler_res_tx
                                .take()
                                .unwrap()
                                .send(Err(OpenError::MissingPlugins(missing_plugins)));

                            return glib::Continue(false);
                        }

                        let _ = handler_res_tx
                            .take()
                            .unwrap()
                            .send(Err(OpenError::Generic(err.get_error().to_string())));

                        return glib::Continue(false);
                    }
                    Element(element_msg) => {
                        let structure = element_msg.get_structure().unwrap();
                        if structure.get_name() == "missing-plugin" {
                            let plugin = structure
                                .get_value("name")
                                .unwrap()
                                .get::<String>()
                                .unwrap()
                                .unwrap();

                            warn!(
                                "{}",
                                gettext("Missing plugin: {}").replacen("{}", &plugin, 1)
                            );
                            this.as_mut().unwrap().missing_plugins.insert(plugin);
                        }
                    }
                    StreamCollection(stream_collection) => {
                        let this = this.as_mut().unwrap();
                        stream_collection
                            .get_stream_collection()
                            .iter()
                            .for_each(|stream| this.info.write().unwrap().add_stream(&stream));
                    }
                    // FIXME really still necessary can't we just use StateChanged?
                    StreamsSelected(_) => {
                        streams_selected = true;
                    }
                    Tag(msg_tag) => {
                        let tags = msg_tag.get_tags();
                        if tags.get_scope() == gst::TagScope::Global {
                            this.as_mut().unwrap().info.write().unwrap().add_tags(&tags);
                        }
                    }
                    Toc(msg_toc) => {
                        // FIXME: use updated
                        let this = this.as_ref().unwrap();
                        if this.info.read().unwrap().toc.is_none() {
                            let (toc, _updated) = msg_toc.get_toc();
                            if toc.get_scope() == gst::TocScope::Global {
                                this.info.write().unwrap().toc = Some(toc);
                            } else {
                                warn!("skipping toc with scope: {:?}", toc.get_scope());
                            }
                        }
                    }
                    AsyncDone(_) => {
                        if streams_selected {
                            let this = this.take().unwrap();

                            let duration = Duration::from_nanos(
                                this.pipeline
                                    .query_duration::<gst::ClockTime>()
                                    .unwrap_or_else(|| 0.into())
                                    .nanoseconds()
                                    .unwrap(),
                            );
                            this.info.write().unwrap().duration = duration;

                            let _ = handler_res_tx.take().unwrap().send(Ok(this));

                            return glib::Continue(false);
                        }
                    }
                    _ => (),
                }

                glib::Continue(true)
            })
            .unwrap();
    }

    fn cleanup(&mut self) {
        if let Some(video_sink) = self.pipeline.get_by_name("video_sink") {
            self.pipeline.remove(&video_sink).unwrap();
        }
    }
}

/// Operations
impl PlaybackPipeline {
    fn register_operations_bus_watch(
        &mut self,
        int_evt_tx: async_mpsc::UnboundedSender<MediaEvent>,
        ext_evt_tx: async_mpsc::UnboundedSender<MediaEvent>,
    ) {
        let bus_watch_src_id = self
            .pipeline
            .get_bus()
            .unwrap()
            .add_watch(move |_, msg| {
                use gst::MessageView::*;

                match msg.view() {
                    AsyncDone(_) => {
                        int_evt_tx.unbounded_send(MediaEvent::AsyncDone).unwrap();
                    }
                    StateChanged(state_changed) => {
                        if state_changed.get_src().unwrap().get_type()
                            == gst::Pipeline::static_type()
                        {
                            int_evt_tx.unbounded_send(MediaEvent::StateChanged).unwrap();
                        }
                    }
                    Eos(_) => {
                        ext_evt_tx.unbounded_send(MediaEvent::Eos).unwrap();
                    }
                    Error(err) => {
                        // FIXME avoid copying the error (use an Rc?)
                        ext_evt_tx
                            .unbounded_send(MediaEvent::Error(err.get_error().to_string()))
                            .unwrap();

                        int_evt_tx
                            .unbounded_send(MediaEvent::Error(err.get_error().to_string()))
                            .unwrap();
                    }
                    _ => (),
                }

                glib::Continue(true)
            })
            .unwrap();

        self.bus_watch_src_id = Some(bus_watch_src_id);
    }

    pub fn current_ts(&self) -> Option<Timestamp> {
        let mut position_query = gst::query::Position::new(gst::Format::Time);
        self.pipeline.query(&mut position_query);
        let position = position_query.get_result().get_value();
        if position < 0 {
            None
        } else {
            Some(position.into())
        }
    }

    /// Purges previous internal messages if any.
    fn purge_int_evt_(&mut self) -> Result<(), PurgeError> {
        while let Ok(event) = self.int_evt_rx.try_next() {
            match event {
                Some(MediaEvent::Error(_)) => {
                    return Err(PurgeError);
                }
                Some(_) => (),
                None => panic!("internal channel terminated"),
            }
        }

        Ok(())
    }

    pub async fn pause(&mut self) -> Result<(), StateChangeError> {
        self.purge_int_evt_()?;

        self.pipeline.set_state(gst::State::Paused)?;

        while let Some(event) = self.int_evt_rx.next().await {
            use MediaEvent::*;
            match event {
                StateChanged => break,
                Error(_) => return Err(StateChangeError),
                _ => (),
            }
        }

        Ok(())
    }

    pub async fn play(&mut self) -> Result<(), StateChangeError> {
        self.purge_int_evt_()?;

        self.pipeline.set_state(gst::State::Playing)?;

        while let Some(event) = self.int_evt_rx.next().await {
            use MediaEvent::*;
            match event {
                StateChanged => break,
                Error(_) => return Err(StateChangeError),
                _ => (),
            }
        }

        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), StateChangeError> {
        if let Some(bus_watch_src_id) = self.bus_watch_src_id.take() {
            glib::source_remove(bus_watch_src_id);
        }

        let res = self.pipeline.set_state(gst::State::Null);
        self.cleanup();
        res?;

        Ok(())
    }

    pub async fn seek(
        &mut self,
        target: Timestamp,
        flags: gst::SeekFlags,
    ) -> Result<(), SeekError> {
        self.purge_int_evt_()?;

        let seek_evt = gst::event::Seek::new(
            1f64,
            gst::SeekFlags::FLUSH | flags,
            gst::SeekType::Set,
            target.into(),
            // FIXME does this remove current segment's end?
            gst::SeekType::None,
            ClockTime::none(),
        );

        let seqnum = seek_evt.get_seqnum();
        if !self.pipeline.send_event(seek_evt) {
            panic!("failed to seek {:?}", seqnum);
        }

        if target >= self.info.read().unwrap().duration {
            return Err(SeekError::Eos);
        }

        while let Some(event) = self.int_evt_rx.next().await {
            use MediaEvent::*;
            match event {
                AsyncDone => break,
                Error(_) => return Err(SeekError::Unrecoverable),
                _ => (),
            }
        }

        Ok(())
    }

    // FIXME move to async
    // FIXME rename to preview range
    // FIXME renderer bin can compute end
    // => use a CustomEvent so that everything is handled from the bin?
    pub fn seek_range(&self, start: Timestamp, end: Timestamp) {
        self.pipeline
            .seek(
                1f64,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                ClockTime::from(start),
                gst::SeekType::Set,
                ClockTime::from(end),
            )
            .unwrap();

        if self.pipeline.set_state(gst::State::Playing).is_err() {
            // FIXME return the error
            warn!("Seeking range: Could not set media in palying state");
        }
    }

    pub async fn select_streams(
        &mut self,
        stream_ids: &[Arc<str>],
    ) -> Result<(), SelectStreamsError> {
        self.purge_int_evt_()?;

        let stream_id_vec: Vec<&str> = stream_ids.iter().map(Borrow::<str>::borrow).collect();
        let select_streams_evt = gst::event::SelectStreams::new(&stream_id_vec);
        self.pipeline.send_event(select_streams_evt);

        self.info
            .write()
            .unwrap()
            .streams
            .select_streams(stream_ids)?;

        Ok(())
    }

    // FIXME remove
    /*
    // Uses sender to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self, mut sender: async_mpsc::Sender<MediaEvent>) {
        let mut pipeline_state = PipelineState::None;
        let info_arc_mtx = Arc::clone(&self.info);
        let dbl_audio_buffer_mtx = Arc::clone(&self.dbl_audio_buffer_mtx);
        let pipeline = self.pipeline.clone();
        self.pipeline
            .get_bus()
            .unwrap()
            .add_watch(move |_, msg| {
                match msg.view() {
                    gst::MessageView::Eos(_) => {
                        dbl_audio_buffer_mtx
                            .lock()
                            .unwrap()
                            .set_state(gst::State::Paused);
                        sender
                            .try_send(MediaEvent::Eos)
                            .expect("Failed to notify UI");
                    }
                    gst::MessageView::Error(err) => {
                        if "sink" == err.get_src().unwrap().get_name() {
                            // Failure detected on a sink, this occurs when the GL sink
                            // can't operate properly
                            // TODO: make sure this only occurs in this particular case
                            sender.try_send(MediaEvent::GLSinkError).unwrap();
                        } else {
                            sender
                                .try_send(MediaEvent::FailedToOpenMedia(
                                    err.get_error().to_string(),
                                ))
                                .unwrap();
                        }
                        return glib::Continue(false);
                    }
                    gst::MessageView::Element(element_msg) => {
                        let structure = element_msg.get_structure().unwrap();
                        if structure.get_name() == "missing-plugin" {
                            sender
                                .try_send(MediaEvent::MissingPlugin(
                                    structure
                                        .get_value("name")
                                        .unwrap()
                                        .get::<String>()
                                        .unwrap()
                                        .unwrap(),
                                ))
                                .unwrap();
                        }
                    }
                    gst::MessageView::AsyncDone(_) => match pipeline_state {
                        PipelineState::Playable(playback_state) => {
                            sender
                                .try_send(MediaEvent::AsyncDone(playback_state))
                                .expect("Failed to notify UI");
                        }
                        PipelineState::StreamsSelected => {
                            pipeline_state = PipelineState::Playable(PlaybackState::Paused);
                            let duration = Duration::from_nanos(
                                pipeline
                                    .query_duration::<gst::ClockTime>()
                                    .unwrap_or_else(|| 0.into())
                                    .nanoseconds()
                                    .unwrap(),
                            );
                            info_arc_mtx
                                .write()
                                .expect("Failed to lock media info while setting duration")
                                .duration = duration;

                            dbl_audio_buffer_mtx
                                .lock()
                                .unwrap()
                                .set_state(gst::State::Paused);

                            sender
                                .try_send(MediaEvent::InitDone)
                                .expect("Failed to notify UI");
                        }
                        _ => (),
                    },
                    gst::MessageView::StateChanged(msg_state_changed) => {
                        if let PipelineState::Playable(_) = pipeline_state {
                            if let Some(source) = msg_state_changed.get_src() {
                                if source.get_type() != gst::Pipeline::static_type() {
                                    return glib::Continue(true);
                                }

                                match msg_state_changed.get_current() {
                                    gst::State::Playing => {
                                        dbl_audio_buffer_mtx
                                            .lock()
                                            .unwrap()
                                            .set_state(gst::State::Playing);
                                        pipeline_state =
                                            PipelineState::Playable(PlaybackState::Playing);
                                    }
                                    gst::State::Paused => {
                                        if msg_state_changed.get_old() != gst::State::Paused {
                                            {
                                                let dbl_audio_buffer =
                                                    &mut dbl_audio_buffer_mtx.lock().unwrap();
                                                dbl_audio_buffer.set_state(gst::State::Paused);
                                                dbl_audio_buffer.refresh();
                                            }
                                            pipeline_state =
                                                PipelineState::Playable(PlaybackState::Paused);
                                            sender.try_send(MediaEvent::ReadyToRefresh).unwrap();
                                        }
                                    }
                                    _ => unreachable!(format!(
                                        "PlaybackPipeline bus inspector, `StateChanged` to {:?}",
                                        msg_state_changed.get_current(),
                                    )),
                                }
                            }
                        }
                    }
                    gst::MessageView::Tag(msg_tag) => match pipeline_state {
                        PipelineState::Playable(_) => (),
                        _ => {
                            let tags = msg_tag.get_tags();
                            if tags.get_scope() == gst::TagScope::Global {
                                info_arc_mtx
                                    .write()
                                    .expect("Failed to lock media info while receiving tags")
                                    .add_tags(&tags);
                            }
                        }
                    },
                    gst::MessageView::Toc(msg_toc) => {
                        match pipeline_state {
                            PipelineState::Playable(_) => (),
                            _ => {
                                // FIXME: use updated
                                if info_arc_mtx.write().unwrap().toc.is_none() {
                                    let (toc, _updated) = msg_toc.get_toc();
                                    if toc.get_scope() == gst::TocScope::Global {
                                        info_arc_mtx.write().unwrap().toc = Some(toc);
                                    } else {
                                        warn!("skipping toc with scope: {:?}", toc.get_scope());
                                    }
                                }
                            }
                        }
                    }
                    gst::MessageView::StreamsSelected(msg_streams_selected) => match pipeline_state
                    {
                        PipelineState::Playable(_) => {
                            sender.try_send(MediaEvent::StreamsSelected).unwrap();
                        }
                        PipelineState::None => {
                            let stream_collection = msg_streams_selected.get_stream_collection();
                            let has_usable_streams = {
                                let info = &mut info_arc_mtx
                                    .write()
                                    .expect("Failed to lock media `info` in `StreamsSelected`");

                                stream_collection
                                    .iter()
                                    .for_each(|stream| info.add_stream(&stream));

                                info.streams.is_audio_selected() || info.streams.is_video_selected()
                            };

                            if has_usable_streams {
                                pipeline_state = PipelineState::StreamsSelected;
                            } else {
                                sender
                                    .try_send(MediaEvent::FailedToOpenMedia(gettext(
                                        "No usable streams could be found.",
                                    )))
                                    .unwrap();
                                return glib::Continue(false);
                            }
                        }
                        PipelineState::StreamsSelected => unreachable!(concat!(
                            "PlaybackPipeline received msg `StreamsSelected` while already ",
                            "being in state `StreamsSelected`",
                        )),
                    },
                    _ => (),
                }

                glib::Continue(true)
            })
            .unwrap();
    }
    */
}
