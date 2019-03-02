use gettextrs::gettext;

use gstreamer as gst;

use gstreamer::{prelude::*, ClockTime};

use glib;
use glib::{Cast, ObjectExt};

use log::{debug, info, warn};

use std::{
    error::Error,
    path::Path,
    sync::{Arc, Mutex, RwLock},
};

use crate::{application::CONFIG, metadata::MediaInfo};

use super::{DoubleAudioBuffer, MediaEvent};

// Buffer size in ns for queues
// This is the max duration that queues can hold
pub const QUEUE_SIZE_NS: u64 = 5_000_000_000u64; // 5s

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

pub struct PlaybackPipeline {
    pipeline: gst::Pipeline,
    decodebin: gst::Element,
    position_query: gst::query::Position<gst::Query>,
    dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,

    pub info: Arc<RwLock<MediaInfo>>,
}

// FIXME: might need to `release_request_pad` on the tee
impl Drop for PlaybackPipeline {
    fn drop(&mut self) {
        if let Some(video_sink) = self.pipeline.get_by_name("video_sink") {
            self.pipeline.remove(&video_sink).unwrap();
        }
    }
}

impl PlaybackPipeline {
    pub fn try_new(
        path: &Path,
        dbl_audio_buffer_mtx: &Arc<Mutex<DoubleAudioBuffer>>,
        video_sink: &Option<gst::Element>,
        sender: glib::Sender<MediaEvent>,
    ) -> Result<PlaybackPipeline, String> {
        info!(
            "{}",
            gettext("Opening {}...").replacen("{}", path.to_str().unwrap(), 1)
        );

        let mut this = PlaybackPipeline {
            pipeline: gst::Pipeline::new("playback_pipeline"),
            decodebin: gst::ElementFactory::make("decodebin3", "decodebin").unwrap(),
            position_query: gst::Query::new_position(gst::Format::Time),
            dbl_audio_buffer_mtx: Arc::clone(dbl_audio_buffer_mtx),

            info: Arc::new(RwLock::new(MediaInfo::new(path))),
        };

        this.pipeline.add(&this.decodebin).unwrap();
        this.build_pipeline(path, video_sink, sender.clone());
        this.register_bus_inspector(sender.clone());

        this.pause().map(|_| this)
    }

    pub fn check_requirements() -> Result<(), String> {
        gst::ElementFactory::make("decodebin3", None).map_or(
            Err(gettext(
                "Missing `decodebin3`\ncheck your gst-plugins-base install",
            )),
            |_| Ok(()),
        )?;
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
            .expect("PlaybackPipeline::play: couldn't lock dbl_audio_buffer_mtx")
            .accept_eos();

        self.pipeline
            .set_state(gst::State::Playing)
            .map(|_| ())
            .map_err(|_| gettext("Could not set media in Playing mode"))
    }

    pub fn pause(&self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Paused)
            .map(|_| ())
            .map_err(|_| gettext("Could not set media in Paused mode"))
    }

    pub fn stop(&self) {
        if self.pipeline.set_state(gst::State::Null).is_err() {
            warn!("could not set media in Null state");
        }
    }

    pub fn seek(&self, position: u64, flags: gst::SeekFlags) {
        self.pipeline
            .seek_simple(gst::SeekFlags::FLUSH | flags, ClockTime::from(position))
            .ok()
            .unwrap();
    }

    pub fn seek_range(&self, start: u64, end: u64) {
        // EOS will be emitted at the end of the range
        // => ignore it so as not to confuse mechnisms that expect the actual end of stream
        self.dbl_audio_buffer_mtx
            .lock()
            .expect("PlaybackPipeline::play: couldn't lock dbl_audio_buffer_mtx")
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
        if self.pipeline.set_state(gst::State::Playing).is_err() {
            warn!("Seeking range: Could not set media in palying state");
            self.dbl_audio_buffer_mtx.lock().unwrap().accept_eos();
        };
    }

    pub fn select_streams(&self, stream_ids: &[Arc<str>]) {
        let stream_id_vec: Vec<&str> = stream_ids.iter().map(|id| id.as_ref()).collect();
        let select_streams_evt = gst::Event::new_select_streams(&stream_id_vec).build();
        self.decodebin.send_event(select_streams_evt);

        {
            let mut info = self.info.write().unwrap();
            info.streams.select_streams(stream_ids);
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
        path: &Path,
        video_sink: &Option<gst::Element>,
        sender: glib::Sender<MediaEvent>,
    ) {
        // From decodebin3's documentation: "Children: multiqueue0"
        let decodebin_as_bin = self.decodebin.clone().downcast::<gst::Bin>().ok().unwrap();
        let decodebin_multiqueue = &decodebin_as_bin.get_children()[0];
        PlaybackPipeline::setup_queue(decodebin_multiqueue);
        // Discard "interleave" as it modifies "max-size-time"
        decodebin_multiqueue
            .set_property("use-interleave", &false)
            .unwrap();

        let file_src = gst::ElementFactory::make("filesrc", "filesrc").unwrap();
        file_src
            .set_property("location", &gst::Value::from(path.to_str().unwrap()))
            .unwrap();
        self.pipeline.add(&file_src).unwrap();
        file_src.link(&self.decodebin).unwrap();

        let audio_sink = gst::ElementFactory::make("autoaudiosink", "audio_playback_sink").unwrap();

        // Prepare pad configuration callback
        let pipeline_clone = self.pipeline.clone();
        let dbl_audio_buffer_mtx = Arc::clone(&self.dbl_audio_buffer_mtx);
        let video_sink = video_sink.clone();
        let sender_mtx = Arc::new(Mutex::new(sender));
        self.decodebin
            .connect_pad_added(move |_decodebin, src_pad| {
                let pipeline = &pipeline_clone;
                let name = src_pad.get_name();

                if name.starts_with("audio_") {
                    PlaybackPipeline::build_audio_pipeline(
                        pipeline,
                        src_pad,
                        &audio_sink,
                        &dbl_audio_buffer_mtx,
                        &sender_mtx,
                    );
                } else if name.starts_with("video_") {
                    if let Some(ref video_sink) = video_sink {
                        PlaybackPipeline::build_video_pipeline(pipeline, src_pad, video_sink);
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
        dbl_audio_buffer_mtx: &Arc<Mutex<DoubleAudioBuffer>>,
        sender_mtx: &Arc<Mutex<glib::Sender<MediaEvent>>>,
    ) {
        let playback_queue = gst::ElementFactory::make("queue", "audio_playback_queue").unwrap();
        PlaybackPipeline::setup_queue(&playback_queue);

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
        PlaybackPipeline::setup_queue(&waveform_queue);

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
            src_pad.link(&tee_sink).unwrap();

            let tee_playback_src_pad = tee.get_request_pad("src_%u").unwrap();
            tee_playback_src_pad.link(&playback_sink_pad).unwrap();

            let tee_waveform_src_pad = tee.get_request_pad("src_%u").unwrap();
            tee_waveform_src_pad.link(&waveform_sink_pad).unwrap();

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
                .expect(
                    "PlaybackPipeline::build_audio_pipeline: couldn't lock dbl_audio_buffer_mtx",
                )
                .set_ref(audio_sink);
        }

        // Pull samples directly off the queue in order to get them as soon as they are available
        // We can't use intermediate elements such as audioconvert because they get paused
        // and block the buffers
        let dbl_audio_buffer_mtx = Arc::clone(dbl_audio_buffer_mtx);
        let sender_mtx = Arc::clone(sender_mtx);
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
                            sender_mtx
                                .lock()
                                .expect("waveform_sink::probe couldn't lock sender_mtx")
                                .send(MediaEvent::ReadyForRefresh)
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
                                sender_mtx
                                    .lock()
                                    .unwrap()
                                    .send(MediaEvent::ReadyForRefresh)
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
        PlaybackPipeline::setup_queue(&queue);
        let convert = gst::ElementFactory::make("videoconvert", None).unwrap();
        let scale = gst::ElementFactory::make("videoscale", None).unwrap();

        let elements = &[&queue, &convert, &scale, video_sink];
        pipeline.add_many(elements).unwrap();
        gst::Element::link_many(elements).unwrap();

        for e in elements {
            // Silently ignore the state sync issues
            // and rely on the PlaybackPipeline state to return an error.
            // Can't use sender to return the error because
            // the bus catches it first.
            let _res = e.sync_state_with_parent();
        }

        let sink_pad = queue.get_static_pad("sink").unwrap();
        src_pad.link(&sink_pad).unwrap();
    }

    // Uses sender to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self, sender: glib::Sender<MediaEvent>) {
        let mut pipeline_state = PipelineState::None;
        let info_arc_mtx = Arc::clone(&self.info);
        let dbl_audio_buffer_mtx = Arc::clone(&self.dbl_audio_buffer_mtx);
        let pipeline = self.pipeline.clone();
        self.pipeline.get_bus().unwrap().add_watch(move |_, msg| {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    {
                        let dbl_audio_buffer = &mut dbl_audio_buffer_mtx.lock().unwrap();
                        dbl_audio_buffer.set_state(gst::State::Paused);
                    }
                    sender
                        .send(MediaEvent::Eos)
                        .expect("Failed to notify UI");
                }
                gst::MessageView::Error(err) => {
                    let msg = if "sink" == err.get_src().unwrap().get_name() {
                        // TODO: make sure this only occurs in this particular case
                        {
                            let mut config = CONFIG.write().expect("Failed to get CONFIG as mut");
                            config.media.is_gl_disabled = true;
                            // Save the config as soon as possible in case something bad occurs
                            // due to the gl sink
                            config.save();
                        }

                        gettext(
"Video rendering hardware acceleration seems broken and has been disabled.\nPlease restart the application.",
                        )
                    } else {
                        err.get_error().description().to_owned()
                    };
                    sender.send(MediaEvent::FailedToOpenMedia(msg)).unwrap();
                    return glib::Continue(false);
                }
                gst::MessageView::Element(element_msg) => {
                    let structure = element_msg.get_structure().unwrap();
                    if structure.get_name() == "missing-plugin" {
                        sender
                            .send(MediaEvent::MissingPlugin(
                                structure
                                    .get_value("name")
                                    .unwrap()
                                    .get::<String>()
                                    .unwrap(),
                            ))
                            .unwrap();
                    }
                }
                gst::MessageView::AsyncDone(_) => match pipeline_state {
                    PipelineState::StreamsSelected => {
                        pipeline_state = PipelineState::Initialized(InitializedState::Paused);
                        let duration = pipeline
                            .query_duration::<gst::ClockTime>()
                            .unwrap_or_else(|| 0.into())
                            .nanoseconds()
                            .unwrap();
                        info_arc_mtx
                            .write()
                            .expect("Failed to lock media info while setting duration")
                            .duration = duration;

                        sender
                            .send(MediaEvent::InitDone)
                            .expect("Failed to notify UI");
                    }
                    PipelineState::Initialized(_) => {
                        sender
                            .send(MediaEvent::AsyncDone)
                            .expect("Failed to notify UI");
                    }
                    _ => (),
                },
                gst::MessageView::StateChanged(msg_state_changed) => {
                    if let PipelineState::Initialized(_) = pipeline_state {
                        if let Some(source) = msg_state_changed.get_src() {
                            if "playback_pipeline" != source.get_name() {
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
                                    if gst::State::Paused != msg_state_changed.get_old() {
                                        {
                                            let dbl_audio_buffer =
                                                &mut dbl_audio_buffer_mtx.lock().unwrap();
                                            dbl_audio_buffer.set_state(gst::State::Paused);
                                        }
                                        pipeline_state =
                                            PipelineState::Initialized(InitializedState::Paused);
                                        sender.send(MediaEvent::ReadyForRefresh).unwrap();
                                    }
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
                        let tags = &mut info_arc_mtx
                            .write()
                            .expect("Failed to lock media info while receiving tags")
                            .tags;
                        *tags = tags.merge(&msg_tag.get_tags(), gst::TagMergeMode::Append);
                    }
                },
                gst::MessageView::Toc(msg_toc) => {
                    match pipeline_state {
                        PipelineState::Initialized(_) => (),
                        _ => {
                            // FIXME: use updated
                            let (toc, _updated) = msg_toc.get_toc();
                            if toc.get_scope() == gst::TocScope::Global {
                                info_arc_mtx.write().unwrap().toc = Some(toc);
                            } else {
                                warn!("skipping toc with scope: {:?}", toc.get_scope());
                            }
                        }
                    }
                }
                gst::MessageView::StreamsSelected(_) => match pipeline_state {
                    PipelineState::Initialized(_) => {
                        let audio_has_changed = info_arc_mtx
                            .read()
                            .expect("Failed to lock media info while comparing streams")
                            .streams
                            .audio_changed;

                        if audio_has_changed {
                            debug!("changing audio stream");
                            let dbl_audio_buffer = &mut dbl_audio_buffer_mtx.lock().unwrap();
                            dbl_audio_buffer.clean_samples();
                        }

                        sender.send(MediaEvent::StreamsSelected).unwrap();
                    }
                    _ => {
                        let has_usable_streams = {
                            let streams = &info_arc_mtx
                                .read()
                                .expect("Failed to lock media info while checking streams")
                                .streams;
                            streams.is_audio_selected() || streams.is_video_selected()
                        };

                        if has_usable_streams {
                            pipeline_state = PipelineState::StreamsSelected;
                        } else {
                            sender
                                .send(MediaEvent::FailedToOpenMedia(gettext(
                                    "No usable streams could be found.",
                                )))
                                .unwrap();
                            return glib::Continue(false);
                        }
                    }
                },
                gst::MessageView::StreamCollection(msg_stream_collection) => {
                    let stream_collection = msg_stream_collection.get_stream_collection();
                    let info = &mut info_arc_mtx.write().unwrap();
                    stream_collection
                        .iter()
                        .for_each(|stream| info.add_stream(&stream));
                }
                _ => (),
            }

            glib::Continue(true)
        });
    }
}
