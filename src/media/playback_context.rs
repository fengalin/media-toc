use gettextrs::gettext;

use gstreamer as gst;

use gstreamer::prelude::*;
use gstreamer::{BinExt, ClockTime, ElementFactory, GstObjectExt, PadExt};

use glib;
use glib::{Cast, ObjectExt};

use gtk;

use std::error::Error;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use metadata::MediaInfo;

use super::{ContextMessage, DoubleAudioBuffer};

// Buffer size in ns for queues
// This is the max duration that queues can hold
pub const QUEUE_SIZE_NS: u64 = 5_000_000_000u64; // 5s

// The video_sink must be created in the main UI thread
// as it contains a gtk::Widget
struct VideoOutput {
    sink: gst::Element,
    widget: gtk::Widget,
}

unsafe impl Sync for VideoOutput {}

lazy_static! {
    static ref VIDEO_OUTPUT: Option<VideoOutput> = {
        let (video_sink, widget_val) =
            // For some reasons, `gtkglsink` seems to interfer with waveform renderding
            // by inducing more jerks than with `gtksink`.
            // For some media, the CPU usage is lower, but at the cost of memory usage
            /*if let Some(gtkglsink) = ElementFactory::make("gtkglsink", "video_sink") {
                debug!("Using gtkglsink");
                let glsinkbin = ElementFactory::make("glsinkbin", "video_sink_bin").unwrap();
                glsinkbin.set_property("sink", &gtkglsink.to_value()).unwrap();
                let widget_val = gtkglsink.get_property("widget");
                (Some(glsinkbin), widget_val.ok())
            } else*/ if let Some(sink) = ElementFactory::make("gtksink", "video_sink") {
                debug!("Using gtksink");
                let widget_val = sink.get_property("widget");
                (Some(sink), widget_val.ok())
            } else {
                (None, None)
            };
        match (video_sink, widget_val) {
            (Some(sink), Some(widget_val)) => {
                match widget_val.get::<gtk::Widget>() {
                    Some(widget) => {
                        Some(VideoOutput {
                            sink,
                            widget,
                        })
                    }
                    None => {
                        error!("{}", gettext("Failed to get Video Widget."));
                        None
                    }
                }
            }
            (Some(_sink), None) => {
                error!("{}", gettext("Failed to get Video Widget."));
                None
            }
            (None, _) => {
                error!("{}", gettext("Couldn't find GStreamer GTK video sink."));
                None
            }
        }
    };
}

#[derive(Clone, Debug, PartialEq)]
pub enum InitializedState {
    Playing,
    Paused,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PipelineState {
    None,
    Initialized(InitializedState),
    StreamsSelected,
}

pub struct PlaybackContext {
    pipeline: gst::Pipeline,
    decodebin: gst::Element,
    position_query: gst::query::Position<gst::Query>,

    dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,

    pub path: PathBuf,
    pub file_name: String,
    pub name: String,

    pub info: Arc<Mutex<MediaInfo>>,
}

// FIXME: might need to `release_request_pad` on the tee
impl Drop for PlaybackContext {
    fn drop(&mut self) {
        if let Some(video_sink) = self.pipeline.get_by_name("video_sink") {
            self.pipeline.remove(&video_sink).unwrap();
        }
    }
}

impl PlaybackContext {
    pub fn new(
        path: PathBuf,
        dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
        ctx_tx: Sender<ContextMessage>,
    ) -> Result<PlaybackContext, String> {
        info!(
            "{}",
            gettext("Opening {}...").replacen("{}", path.to_str().unwrap(), 1)
        );

        let file_name = String::from(path.file_name().unwrap().to_str().unwrap());

        let mut this = PlaybackContext {
            pipeline: gst::Pipeline::new("pipeline"),
            decodebin: gst::ElementFactory::make("decodebin3", "decodebin").unwrap(),
            position_query: gst::Query::new_position(gst::Format::Time),

            dbl_audio_buffer_mtx: Arc::clone(&dbl_audio_buffer_mtx),

            file_name: file_name.clone(),
            name: String::from(path.file_stem().unwrap().to_str().unwrap()),
            path,

            info: Arc::new(Mutex::new(MediaInfo::new())),
        };

        this.pipeline.add(&this.decodebin).unwrap();

        this.info
            .lock()
            .expect("PlaybackContext::new failed to lock media info")
            .file_name = file_name;

        this.build_pipeline(
            ctx_tx.clone(),
            dbl_audio_buffer_mtx,
            (*VIDEO_OUTPUT)
                .as_ref()
                .map(|video_output| video_output.sink.clone()),
        );
        this.register_bus_inspector(ctx_tx);

        match this.pause() {
            Ok(_) => Ok(this),
            Err(error) => Err(error),
        }
    }

    pub fn check_requirements() -> Result<(), String> {
        gst::ElementFactory::make("decodebin3", None)
            .map_or(
                Err(gettext(
                    "Missing `decodebin3`\ncheck your gst-plugins-base install",
                )),
                |_| Ok(()),
            )
            .and_then(|_| {
                gst::ElementFactory::make("gtksink", None).map_or_else(
                    || {
                        let (major, minor, _micro, _nano) = gst::version();
                        let (variant1, variant2) = if major >= 1 && minor >= 14 {
                            ("gstreamer1-plugins-base", "gstreamer1.0-plugins-base")
                        } else {
                            (
                                "gstreamer1-plugins-bad-free-gtk",
                                "gstreamer1.0-plugins-bad",
                            )
                        };
                        Err(format!(
                            "{} {}\n{}",
                            gettext("Couldn't find GStreamer GTK video sink."),
                            gettext("Video playback will be disabled."),
                            gettext("Please install {} or {}, depending on your distribution.")
                                .replacen("{}", variant1, 1)
                                .replacen("{}", variant2, 1),
                        ))
                    },
                    |_| Ok(()),
                )
            })
    }

    pub fn get_video_widget() -> Option<gtk::Widget> {
        (*VIDEO_OUTPUT)
            .as_ref()
            .map(|video_output| video_output.widget.clone())
    }

    pub fn get_position(&mut self) -> u64 {
        self.pipeline.query(&mut self.position_query);
        self.position_query.get_result().get_value() as u64
    }

    pub fn get_state(&self) -> gst::State {
        let (_, current, _) = self.pipeline.get_state(ClockTime::from(10_000_000));
        current
    }

    pub fn play(&mut self) -> Result<(), String> {
        self.dbl_audio_buffer_mtx
            .lock()
            .expect("PlaybackContext::play: couldn't lock dbl_audio_buffer_mtx")
            .accept_eos();

        if self.pipeline.set_state(gst::State::Playing) == gst::StateChangeReturn::Failure {
            return Err(gettext("Could not set media in playing state."));
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<(), String> {
        if self.pipeline.set_state(gst::State::Paused) == gst::StateChangeReturn::Failure {
            return Err(gettext("Could not set media in paused state."));
        }
        Ok(())
    }

    pub fn stop(&self) {
        if self.pipeline.set_state(gst::State::Null) == gst::StateChangeReturn::Failure {
            warn!("could not set media in Null state");
        }
    }

    pub fn seek(&self, position: u64, accurate: bool) {
        let flags = gst::SeekFlags::FLUSH | if accurate {
            gst::SeekFlags::ACCURATE
        } else {
            gst::SeekFlags::KEY_UNIT
        };
        self.pipeline
            .seek_simple(flags, ClockTime::from(position))
            .ok()
            .unwrap();
    }

    pub fn seek_range(&self, start: u64, end: u64) {
        // EOS will be emitted at the end of the range
        // => ignore it so as not to confuse mechnisms that expect the actual end of stream
        self.dbl_audio_buffer_mtx
            .lock()
            .expect("PlaybackContext::play: couldn't lock dbl_audio_buffer_mtx")
            .ignore_eos();

        self.pipeline
            .seek(
                1f64,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                ClockTime::from(start),
                gst::SeekType::Set,
                ClockTime::from(end),
            )
            .ok()
            .unwrap();
        if self.pipeline.set_state(gst::State::Playing) == gst::StateChangeReturn::Failure {
            warn!("Seeking range: Could not set media in palying state");
            self.dbl_audio_buffer_mtx.lock().unwrap().accept_eos();
        }
    }

    pub fn select_streams(&self, stream_ids: &[String]) {
        let stream_ids: Vec<&str> = stream_ids.iter().map(|id| id.as_str()).collect();
        let select_streams_evt = gst::Event::new_select_streams(&stream_ids).build();
        self.decodebin.send_event(select_streams_evt);

        {
            let mut info = self.info.lock().unwrap();
            info.streams.select_streams(&stream_ids);
        }
    }

    fn setup_queue(queue: &gst::Element) {
        queue.set_property("max-size-bytes", &0u32).unwrap();
        queue.set_property("max-size-buffers", &0u32).unwrap();
        queue.set_property("max-size-time", &QUEUE_SIZE_NS).unwrap();

        #[cfg(feature = "trace-playback-queues")]
        queue
            .connect("overrun", false, |args| {
                let queue = args[0].get::<gst::Element>().unwrap();
                warn!(
                    "OVERRUN {} (max-sizes: bytes {:?}, buffers {:?}, time {:?})",
                    queue.get_name(),
                    queue
                        .get_property("max-size-bytes")
                        .unwrap()
                        .get::<u32>()
                        .unwrap(),
                    queue
                        .get_property("max-size-buffers")
                        .unwrap()
                        .get::<u32>()
                        .unwrap(),
                    queue
                        .get_property("max-size-time")
                        .unwrap()
                        .get::<u64>()
                        .unwrap(),
                );
                None
            })
            .ok()
            .unwrap();
        #[cfg(feature = "trace-playback-queues")]
        queue
            .connect("underrun", false, |args| {
                let queue = args[0].get::<gst::Element>().unwrap();
                warn!(
                    "UNDERRUN {} (max-sizes: bytes {:?}, buffers {:?}, time {:?})",
                    queue.get_name(),
                    queue
                        .get_property("max-size-bytes")
                        .unwrap()
                        .get::<u32>()
                        .unwrap(),
                    queue
                        .get_property("max-size-buffers")
                        .unwrap()
                        .get::<u32>()
                        .unwrap(),
                    queue
                        .get_property("max-size-time")
                        .unwrap()
                        .get::<u64>()
                        .unwrap(),
                );
                None
            })
            .ok()
            .unwrap();
    }

    fn build_pipeline(
        &mut self,
        ctx_tx: Sender<ContextMessage>,
        dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
        video_sink: Option<gst::Element>,
    ) {
        // From decodebin3's documentation: "Children: multiqueue0"
        let decodebin_as_bin = self.decodebin.clone().downcast::<gst::Bin>().ok().unwrap();
        let decodebin_multiqueue = &decodebin_as_bin.get_children()[0];
        PlaybackContext::setup_queue(decodebin_multiqueue);
        // Discard "interleave" as it modifies "max-size-time"
        decodebin_multiqueue
            .set_property("use-interleave", &false)
            .unwrap();

        let file_src = gst::ElementFactory::make("filesrc", "filesrc").unwrap();
        file_src
            .set_property("location", &gst::Value::from(self.path.to_str().unwrap()))
            .unwrap();
        self.pipeline.add(&file_src).unwrap();
        file_src.link(&self.decodebin).unwrap();

        let audio_sink = gst::ElementFactory::make("autoaudiosink", "audio_playback_sink").unwrap();

        // Prepare pad configuration callback
        let pipeline_clone = self.pipeline.clone();
        let ctx_tx_mtx = Arc::new(Mutex::new(ctx_tx));
        self.decodebin
            .connect_pad_added(move |_decodebin, src_pad| {
                let pipeline = &pipeline_clone;
                let name = src_pad.get_name();

                if name.starts_with("audio_") {
                    PlaybackContext::build_audio_pipeline(
                        pipeline,
                        src_pad,
                        &audio_sink,
                        Arc::clone(&ctx_tx_mtx),
                        Arc::clone(&dbl_audio_buffer_mtx),
                    );
                } else if name.starts_with("video_") {
                    if let Some(ref video_sink) = video_sink {
                        PlaybackContext::build_video_pipeline(pipeline, src_pad, video_sink);
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
        ctx_tx_mtx: Arc<Mutex<Sender<ContextMessage>>>,
        dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
    ) {
        let playback_queue = gst::ElementFactory::make("queue", "audio_playback_queue").unwrap();
        PlaybackContext::setup_queue(&playback_queue);

        let playback_convert =
            gst::ElementFactory::make("audioconvert", "playback_audioconvert").unwrap();
        let playback_resample =
            gst::ElementFactory::make("audioresample", "playback_audioresample").unwrap();
        let playback_sink_pad = playback_queue.get_static_pad("sink").unwrap();
        let playback_elements = &[
            &playback_queue,
            &playback_convert,
            &playback_resample,
            audio_sink,
        ];

        let waveform_queue = gst::ElementFactory::make("queue2", "waveform_queue").unwrap();
        PlaybackContext::setup_queue(&waveform_queue);

        let waveform_sink = gst::ElementFactory::make("fakesink", "waveform_sink").unwrap();
        let waveform_sink_pad = waveform_queue.get_static_pad("sink").unwrap();

        {
            let waveform_elements = &[&waveform_queue, &waveform_sink];
            let tee = gst::ElementFactory::make("tee", "audio_tee").unwrap();
            let mut elements = vec![&tee];
            elements.extend_from_slice(playback_elements);
            elements.extend_from_slice(waveform_elements);
            pipeline.add_many(elements.as_slice()).unwrap();

            gst::Element::link_many(playback_elements).unwrap();
            gst::Element::link_many(waveform_elements).unwrap();

            let tee_sink = tee.get_static_pad("sink").unwrap();
            assert_eq!(src_pad.link(&tee_sink), gst::PadLinkReturn::Ok);

            let tee_playback_src_pad = tee.get_request_pad("src_%u").unwrap();
            assert_eq!(
                tee_playback_src_pad.link(&playback_sink_pad),
                gst::PadLinkReturn::Ok
            );

            let tee_waveform_src_pad = tee.get_request_pad("src_%u").unwrap();
            assert_eq!(
                tee_waveform_src_pad.link(&waveform_sink_pad),
                gst::PadLinkReturn::Ok
            );

            for e in elements {
                e.sync_state_with_parent().unwrap();
            }
        }

        // FIXME: build a dedicated plugin?
        // get samples as fast as possible
        waveform_sink
            .set_property("sync", &gst::Value::from(&false))
            .unwrap();
        // and don't block pipeline when switching state
        waveform_sink
            .set_property("async", &gst::Value::from(&false))
            .unwrap();

        {
            dbl_audio_buffer_mtx
                .lock()
                .expect("PlaybackContext::build_audio_pipeline: couldn't lock dbl_audio_buffer_mtx")
                .set_ref(audio_sink);
        }

        // Pull samples directly off the queue in order to get them as soon as they are available
        // We can't use intermediate elements such as audioconvert because they get paused
        // and block the buffers
        let pad_probe_filter = gst::PadProbeType::BUFFER | gst::PadProbeType::EVENT_BOTH;
        waveform_sink_pad.add_probe(pad_probe_filter, move |_pad, probe_info| {
            if let Some(ref mut data) = probe_info.data {
                match *data {
                    gst::PadProbeData::Buffer(ref buffer) => {
                        let must_notify = dbl_audio_buffer_mtx
                            .lock()
                            .expect("waveform_sink::probe couldn't lock dbl_audio_buffer")
                            .push_gst_buffer(buffer);

                        if must_notify {
                            ctx_tx_mtx
                                .lock()
                                .expect("waveform_sink::probe couldn't lock ctx_tx_mtx")
                                .send(ContextMessage::ReadyForRefresh)
                                .expect("Failed to notify UI");
                        }

                        if !buffer.get_flags().intersects(gst::BufferFlags::DISCONT) {
                            return gst::PadProbeReturn::Handled;
                        }
                    }
                    gst::PadProbeData::Event(ref event) => {
                        match event.view() {
                            // TODO: handle FlushStart / FlushStop
                            gst::EventView::Caps(caps_event) => {
                                dbl_audio_buffer_mtx
                                    .lock()
                                    .unwrap()
                                    .set_caps(caps_event.get_caps());
                            }
                            gst::EventView::Eos(_) => {
                                dbl_audio_buffer_mtx.lock().unwrap().handle_eos();
                                ctx_tx_mtx
                                    .lock()
                                    .unwrap()
                                    .send(ContextMessage::ReadyForRefresh)
                                    .unwrap();
                            }
                            gst::EventView::Segment(segment_event) => {
                                dbl_audio_buffer_mtx
                                    .lock()
                                    .unwrap()
                                    .have_gst_segment(segment_event.get_segment());
                            }
                            _ => (),
                        }
                    }
                    _ => (),
                }
            }
            gst::PadProbeReturn::Ok
        });
    }

    fn build_video_pipeline(
        pipeline: &gst::Pipeline,
        src_pad: &gst::Pad,
        video_sink: &gst::Element,
    ) {
        let queue = gst::ElementFactory::make("queue", "video_queue").unwrap();
        PlaybackContext::setup_queue(&queue);
        let convert = gst::ElementFactory::make("videoconvert", None).unwrap();
        let scale = gst::ElementFactory::make("videoscale", None).unwrap();

        let elements = &[&queue, &convert, &scale, video_sink];
        pipeline.add_many(elements).unwrap();
        gst::Element::link_many(elements).unwrap();

        for e in elements {
            e.sync_state_with_parent().unwrap();
        }

        let sink_pad = queue.get_static_pad("sink").unwrap();
        assert_eq!(src_pad.link(&sink_pad), gst::PadLinkReturn::Ok);
    }

    // Uses ctx_tx to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self, ctx_tx: Sender<ContextMessage>) {
        let mut pipeline_state = PipelineState::None;
        let info_arc_mtx = Arc::clone(&self.info);
        let dbl_audio_buffer_mtx = Arc::clone(&self.dbl_audio_buffer_mtx);
        let pipeline = self.pipeline.clone();
        self.pipeline.get_bus().unwrap().add_watch(move |_, msg| {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    {
                        let dbl_audio_buffer =
                            &mut dbl_audio_buffer_mtx.lock().unwrap();
                        dbl_audio_buffer.set_state(gst::State::Paused);
                    }
                    ctx_tx
                        .send(ContextMessage::Eos)
                        .expect("Failed to notify UI");
                }
                gst::MessageView::Error(err) => {
                    ctx_tx
                        .send(ContextMessage::FailedToOpenMedia(
                            err.get_error().description().to_owned(),
                        ))
                        .unwrap();
                    return glib::Continue(false);
                }
                gst::MessageView::Element(element_msg) => {
                    let structure = element_msg.get_structure().unwrap();
                    if structure.get_name() == "missing-plugin" {
                        ctx_tx
                            .send(ContextMessage::MissingPlugin(
                                structure.get_value("name").unwrap().get::<String>().unwrap(),
                            ))
                            .unwrap();
                    }
                }
                gst::MessageView::AsyncDone(_) => match pipeline_state {
                    PipelineState::StreamsSelected => {
                        pipeline_state = PipelineState::Initialized(InitializedState::Paused);
                        {
                            let info = &mut info_arc_mtx
                                .lock()
                                .expect("Failed to lock media info while setting duration");
                            info.duration = pipeline
                                .query_duration::<gst::ClockTime>()
                                .unwrap_or_else(|| 0.into())
                                .nanoseconds()
                                .unwrap();
                        }
                        ctx_tx
                            .send(ContextMessage::InitDone)
                            .expect("Failed to notify UI");
                    }
                    PipelineState::Initialized(_) => {
                        ctx_tx
                            .send(ContextMessage::AsyncDone)
                            .expect("Failed to notify UI");
                    }
                    _ => (),
                },
                gst::MessageView::StateChanged(msg_state_changed) => {
                    if let PipelineState::Initialized(_) = pipeline_state {
                        if let Some(source) = msg_state_changed.get_src() {
                            if "pipeline" != source.get_name() {
                                return glib::Continue(true);
                            }

                            match msg_state_changed.get_current() {
                                gst::State::Playing => {
                                    {
                                        let dbl_audio_buffer =
                                            &mut dbl_audio_buffer_mtx.lock().unwrap();
                                        dbl_audio_buffer.set_state(gst::State::Playing);
                                    }
                                    pipeline_state =
                                        PipelineState::Initialized(InitializedState::Playing);
                                }
                                gst::State::Paused => {
                                    {
                                        let dbl_audio_buffer =
                                            &mut dbl_audio_buffer_mtx.lock().unwrap();
                                        dbl_audio_buffer.set_state(gst::State::Paused);
                                    }
                                    pipeline_state =
                                        PipelineState::Initialized(InitializedState::Paused);
                                }
                                _ => {
                                    {
                                        let dbl_audio_buffer =
                                            &mut dbl_audio_buffer_mtx.lock().unwrap();
                                        dbl_audio_buffer.set_state(gst::State::Null);
                                    }
                                    pipeline_state = PipelineState::None;
                                }
                            }
                        }
                    }
                }
                gst::MessageView::Tag(msg_tag) => match pipeline_state {
                    PipelineState::Initialized(_) => (),
                    _ => {
                        let info = &mut info_arc_mtx
                            .lock()
                            .expect("Failed to lock media info while reading tags");
                        info.tags = info.tags
                            .merge(&msg_tag.get_tags(), gst::TagMergeMode::Replace);
                    }
                },
                gst::MessageView::Toc(msg_toc) => {
                    match pipeline_state {
                        PipelineState::Initialized(_) => (),
                        _ => {
                            // FIXME: use updated
                            let (toc, _updated) = msg_toc.get_toc();
                            if toc.get_scope() == gst::TocScope::Global {
                                let info = &mut info_arc_mtx.lock().unwrap();
                                info.toc = Some(toc);
                            } else {
                                warn!("skipping toc with scope: {:?}", toc.get_scope());
                            }
                        }
                    }
                }
                gst::MessageView::StreamsSelected(_) => match pipeline_state {
                    PipelineState::Initialized(_) => {
                        ctx_tx.send(ContextMessage::StreamsSelected).unwrap();
                    }
                    _ => pipeline_state = PipelineState::StreamsSelected,
                },
                gst::MessageView::StreamCollection(msg_stream_collection) => {
                    let stream_collection = msg_stream_collection.get_stream_collection();
                    let info = &mut info_arc_mtx.lock().unwrap();
                    stream_collection
                        .iter()
                        .for_each(|stream| info.streams.add_stream(&stream));
                }
                _ => (),
            }

            glib::Continue(true)
        });
    }
}
