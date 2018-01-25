extern crate gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::{BinExt, Caps, ClockTime, ElementFactory, GstObjectExt, PadExt, QueryView,
                TocEntryType, TocScope};

extern crate gstreamer_app as gst_app;
extern crate gstreamer_audio as gst_audio;

extern crate glib;
use glib::{Cast, ObjectExt};

extern crate gtk;

extern crate lazy_static;

use std::path::PathBuf;

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use std::i32;

use metadata::{Chapter, MediaInfo, Timestamp};

use super::{ContextMessage, DoubleAudioBuffer};

// Buffer size in ns for queues
// This is the max duration that queues can hold
const QUEUE_SIZE_NS: u64 = 5_000_000_000u64; // 5s

// The video_sink must be created in the main UI thread
// as it contains a gtk::Widget
// GLGTKSink not used because it causes flickerings on Xorg systems.
lazy_static! {
    static ref VIDEO_SINK: gst::Element =
        ElementFactory::make("gtksink", "video_sink")
            .expect(concat!(
                "Couldn't find GStreamer GTK video sink. Please install ",
                "gstreamer1-plugins-bad-free-gtk or gstreamer1.0-plugins-bad, ",
                "depenging on your distribution."
            ));
}

#[derive(Clone, Debug, PartialEq)]
pub enum PipelineState {
    None,
    Initialized,
    StreamsStarted,
    StreamsSelected,
}

pub struct PlaybackContext {
    pipeline: gst::Pipeline,
    decodebin: gst::Element,
    position_element: Option<gst::Element>,
    position_query: gst::Query,

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
        println!("\n* Opening {:?}...", path);

        let file_name = String::from(path.file_name().unwrap().to_str().unwrap());

        let mut this = PlaybackContext {
            pipeline: gst::Pipeline::new("pipeline"),
            decodebin: gst::ElementFactory::make("decodebin3", None).unwrap(),
            position_element: None,
            position_query: gst::Query::new_position(gst::Format::Time),

            dbl_audio_buffer_mtx: Arc::clone(&dbl_audio_buffer_mtx),

            file_name: file_name.clone(),
            name: String::from(path.file_stem().unwrap().to_str().unwrap()),
            path: path,

            info: Arc::new(Mutex::new(MediaInfo::new())),
        };

        this.pipeline.add(&this.decodebin).unwrap();

        this.info
            .lock()
            .expect("PlaybackContext::new failed to lock media info")
            .file_name = file_name;

        this.build_pipeline(dbl_audio_buffer_mtx, (*VIDEO_SINK).clone());
        this.register_bus_inspector(ctx_tx);

        match this.pause() {
            Ok(_) => Ok(this),
            Err(error) => Err(error),
        }
    }

    pub fn get_video_widget() -> gtk::Widget {
        let widget_val = (*VIDEO_SINK).get_property("widget").unwrap();
        widget_val
            .get::<gtk::Widget>()
            .expect("Failed to get GstGtkWidget glib::Value as gtk::Widget")
    }

    pub fn get_position(&mut self) -> u64 {
        let pipeline = self.pipeline.clone();
        self.position_element
            .get_or_insert_with(|| {
                if let Some(video) = pipeline.get_by_name("video_sink") {
                    video
                } else if let Some(audio) = pipeline.get_by_name("audio_playback_sink") {
                    audio
                } else {
                    panic!("No sink in pipeline");
                }
            })
            .query(self.position_query.get_mut().unwrap());
        match self.position_query.view() {
            QueryView::Position(ref position) => position.get_result().get_value() as u64,
            _ => unreachable!(),
        }
    }

    pub fn get_duration(&self) -> u64 {
        self.pipeline
            .query_duration::<gst::ClockTime>()
            .unwrap_or_else(|| 0.into())
            .nanoseconds()
            .unwrap()
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
            return Err("Could not set media in palying state".into());
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<(), String> {
        if self.pipeline.set_state(gst::State::Paused) == gst::StateChangeReturn::Failure {
            return Err("Could not set media in Paused state".into());
        }
        Ok(())
    }

    pub fn stop(&self) {
        if self.pipeline.set_state(gst::State::Null) == gst::StateChangeReturn::Failure {
            println!("Could not set media in Null state");
            //return Err("could not set media in Null state".into());
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
            println!("Seeking range: Could not set media in palying state");
            self.dbl_audio_buffer_mtx
                .lock()
                .expect("PlaybackContext::play: couldn't lock dbl_audio_buffer_mtx")
                .accept_eos();
        }
    }

    pub fn select_streams(&self, stream_ids: &Vec<String>) {
        let stream_ids: Vec<&str> = stream_ids.iter().map(|id| id.as_str()).collect();
        let select_streams_evt = gst::Event::new_select_streams(&stream_ids).build();
        self.decodebin.send_event(select_streams_evt);

        {
            let mut info = self.info
                .lock()
                .expect("MainController::select_streams failed to lock info");
            info.streams.select_streams(&stream_ids);
        }
    }

    fn setup_queue(queue: &gst::Element) {
        queue
            .set_property("max-size-bytes", &0u32)
            .unwrap();
        queue
            .set_property("max-size-buffers", &0u32)
            .unwrap();
        queue
            .set_property("max-size-time", &QUEUE_SIZE_NS)
            .unwrap();

        #[cfg(feature = "trace-playback-queues")]
        queue
            .connect("overrun", false, |args| {
                let queue = args[0].get::<gst::Element>().unwrap();
                println!("\n/!\\ OVERRUN {} (max-sizes: bytes {:?}, buffers {:?}, time {:?})",
                    queue.get_name(),
                    queue.get_property("max-size-bytes").unwrap().get::<u32>().unwrap(),
                    queue.get_property("max-size-buffers").unwrap().get::<u32>().unwrap(),
                    queue.get_property("max-size-time").unwrap().get::<u64>().unwrap(),
                );
                None
            })
            .ok()
            .unwrap();
        #[cfg(feature = "trace-playback-queues")]
        queue
            .connect("underrun", false, |args| {
                let queue = args[0].get::<gst::Element>().unwrap();
                println!("\n/!\\ UNDERRUN {} (max-sizes: bytes {:?}, buffers {:?}, time {:?})",
                    queue.get_name(),
                    queue.get_property("max-size-bytes").unwrap().get::<u32>().unwrap(),
                    queue.get_property("max-size-buffers").unwrap().get::<u32>().unwrap(),
                    queue.get_property("max-size-time").unwrap().get::<u64>().unwrap(),
                );
                None
            })
            .ok()
            .unwrap();
    }

    // TODO: handle errors
    fn build_pipeline(
        &mut self,
        dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
        video_sink: gst::Element,
    ) {
        // From decodebin3's documentation: "Children: multiqueue0"
        let decodebin_as_bin = self.decodebin.clone().downcast::<gst::Bin>().ok().unwrap();
        let decodebin_multiqueue = &decodebin_as_bin.get_children()[0];
        PlaybackContext::setup_queue(decodebin_multiqueue);
        // Discard "interleave" as it modifies "max-size-time"
        decodebin_multiqueue
            .set_property("use-interleave", &false)
            .unwrap();

        let file_src = gst::ElementFactory::make("filesrc", None).unwrap();
        file_src
            .set_property("location", &gst::Value::from(self.path.to_str().unwrap()))
            .unwrap();
        self.pipeline.add(&file_src).unwrap();
        file_src.link(&self.decodebin).unwrap();

        let audio_sink = gst::ElementFactory::make("autoaudiosink", "audio_playback_sink").unwrap();

        // Prepare pad configuration callback
        let pipeline_clone = self.pipeline.clone();
        self.decodebin.connect_pad_added(move |_decodebin, src_pad| {
            let pipeline = &pipeline_clone;
            let name = src_pad.get_name();

            if name.starts_with("audio_") {
                PlaybackContext::build_audio_pipeline(
                    pipeline,
                    src_pad,
                    &audio_sink,
                    Arc::clone(&dbl_audio_buffer_mtx),
                );
            } else if name.starts_with("video_") {
                PlaybackContext::build_video_pipeline(pipeline, src_pad, &video_sink);
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
        dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
    ) {
        let playback_queue = gst::ElementFactory::make("queue", "audio_playback_queue").unwrap();
        PlaybackContext::setup_queue(&playback_queue);

        let playback_convert = gst::ElementFactory::make("audioconvert", None).unwrap();
        let playback_resample = gst::ElementFactory::make("audioresample", None).unwrap();
        let playback_sink_sink_pad = playback_queue.get_static_pad("sink").unwrap();
        let playback_elements = &[
            &playback_queue,
            &playback_convert,
            &playback_resample,
            audio_sink,
        ];

        let waveform_queue = gst::ElementFactory::make("queue", "waveform_queue").unwrap();
        PlaybackContext::setup_queue(&waveform_queue);

        let waveform_convert = gst::ElementFactory::make("audioconvert", None).unwrap();
        let waveform_sink = gst::ElementFactory::make("appsink", "waveform_sink").unwrap();
        let waveform_sink_sink_pad = waveform_queue.get_static_pad("sink").unwrap();

        {
            let waveform_elements = &[&waveform_queue, &waveform_convert, &waveform_sink];
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
                tee_playback_src_pad.link(&playback_sink_sink_pad),
                gst::PadLinkReturn::Ok
            );

            let tee_waveform_src_pad = tee.get_request_pad("src_%u").unwrap();
            assert_eq!(
                tee_waveform_src_pad.link(&waveform_sink_sink_pad),
                gst::PadLinkReturn::Ok
            );

            for e in elements {
                e.sync_state_with_parent().unwrap();
            }
        }

        let appsink = waveform_sink.dynamic_cast::<gst_app::AppSink>().unwrap();
        appsink.set_caps(&Caps::new_simple(
            "audio/x-raw",
            &[
                ("format", &gst_audio::AUDIO_FORMAT_S16.to_string()),
                ("layout", &"interleaved"),
                ("channels", &gst::IntRange::<i32>::new(1, i32::MAX)),
                ("rate", &gst::IntRange::<i32>::new(1, i32::MAX)),
            ],
        ));

        // get samples as fast as possible
        appsink
            .set_property("sync", &gst::Value::from(&false))
            .unwrap();
        // and don't block pipeline when switching state
        appsink
            .set_property("async", &gst::Value::from(&false))
            .unwrap();

        {
            dbl_audio_buffer_mtx
                .lock()
                .expect("PlaybackContext::build_audio_pipeline: couldn't lock dbl_audio_buffer_mtx")
                .set_ref(audio_sink);
        }

        let dbl_audio_buffer_mtx_caps = Arc::clone(&dbl_audio_buffer_mtx);
        waveform_sink_sink_pad.connect_property_caps_notify(move |pad| {
            if let Some(caps) = pad.get_current_caps() {
                #[cfg(feature = "trace-audio-caps")]
                println!("\nGot new {:#?}", caps);

                dbl_audio_buffer_mtx_caps
                    .lock()
                    .expect(
                        "PlaybackContext::property_caps_notify couldn't lock dbl_audio_buffer_mtx"
                    )
                    .set_caps(&caps);
            }
        });

        let dbl_audio_buffer_mtx_eos = Arc::clone(&dbl_audio_buffer_mtx);
        let dbl_audio_buffer_mtx_pre = Arc::clone(&dbl_audio_buffer_mtx);
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::new()
                .eos(move |_| {
                    dbl_audio_buffer_mtx_eos
                        .lock()
                        .expect("appsink: eos: couldn't lock dbl_audio_buffer")
                        .handle_eos();
                })
                .new_preroll(move |appsink| match appsink.pull_preroll() {
                    Some(sample) => {
                        dbl_audio_buffer_mtx_pre
                            .lock()
                            .expect("appsink: new_preroll: couldn't lock dbl_audio_buffer")
                            .preroll_gst_sample(&sample);
                        gst::FlowReturn::Ok
                    }
                    None => gst::FlowReturn::Eos,
                })
                .new_sample(move |appsink| match appsink.pull_sample() {
                    Some(sample) => {
                        {
                            dbl_audio_buffer_mtx
                                .lock()
                                .expect("appsink: new_samples: couldn't lock dbl_audio_buffer")
                                .push_gst_sample(&sample);
                        }
                        gst::FlowReturn::Ok
                    }
                    None => gst::FlowReturn::Eos,
                })
                .build(),
        );
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
        self.pipeline.get_bus().unwrap().add_watch(move |_, msg| {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    ctx_tx
                        .send(ContextMessage::Eos)
                        .expect("Failed to notify UI");
                }
                gst::MessageView::Error(err) => {
                    eprintln!(
                        "Error from {}: {} ({:?})",
                        msg.get_src()
                            .map(|s| s.get_path_string(),)
                            .unwrap_or_else(|| String::from("None"),),
                        err.get_error(),
                        err.get_debug()
                    );
                    ctx_tx
                        .send(ContextMessage::FailedToOpenMedia)
                        .expect("Failed to notify UI");
                    return glib::Continue(false);
                }
                gst::MessageView::AsyncDone(_) => {
                    if pipeline_state == PipelineState::StreamsSelected {
                        pipeline_state = PipelineState::Initialized;
                        ctx_tx
                            .send(ContextMessage::InitDone)
                            .expect("Failed to notify UI");
                    } else if pipeline_state == PipelineState::Initialized {
                        ctx_tx
                            .send(ContextMessage::AsyncDone)
                            .expect("Failed to notify UI");
                    }
                }
                gst::MessageView::Tag(msg_tag) => {
                    if pipeline_state != PipelineState::Initialized {
                        let info = &mut info_arc_mtx
                            .lock()
                            .expect("Failed to lock media info while reading toc data");
                        info.tags = info.tags
                            .merge(&msg_tag.get_tags(), gst::TagMergeMode::Replace);
                    }
                }
                gst::MessageView::Toc(msg_toc) => {
                    if pipeline_state != PipelineState::Initialized {
                        // FIXME: use updated
                        let (toc, _updated) = msg_toc.get_toc();
                        if toc.get_scope() == TocScope::Global {
                            PlaybackContext::add_toc(&toc, &info_arc_mtx);
                        } else {
                            println!("Warning: Skipping toc with scope: {:?}", toc.get_scope());
                        }
                    }
                }
                gst::MessageView::StreamStart(_) => {
                    if pipeline_state == PipelineState::None {
                        pipeline_state = PipelineState::StreamsStarted;
                    }
                }
                gst::MessageView::StreamsSelected(_) => {
                    if pipeline_state == PipelineState::Initialized {
                        ctx_tx
                            .send(ContextMessage::StreamsSelected)
                            .expect("Failed to notify UI");
                    } else {
                        pipeline_state = PipelineState::StreamsSelected;
                    }
                }
                gst::MessageView::StreamCollection(msg_stream_collection) => {
                    let stream_collection = msg_stream_collection.get_stream_collection();
                    let info = &mut info_arc_mtx
                        .lock()
                        .expect("Failed to lock media info while initializing audio stream");
                    stream_collection.iter().for_each(|stream| info.streams.add_stream(&stream));
                }
                _ => (),
            }

            glib::Continue(true)
        });
    }

    fn add_toc(toc: &gst::Toc, info_arc_mtx: &Arc<Mutex<MediaInfo>>) {
        let info = &mut info_arc_mtx
            .lock()
            .expect("Failed to lock media info while reading toc data");
        if info.chapters.is_empty() {
            // chapters not retrieved yet
            // TODO: check if there are medias with some sort of
            // incremental tocs (not likely for files)
            // or maybe the updated flag (_ above) should be used

            for entry in toc.get_entries() {
                if entry.get_entry_type() == TocEntryType::Edition {
                    for sub_entry in entry.get_sub_entries() {
                        if sub_entry.get_entry_type() == TocEntryType::Chapter {
                            if let Some((start, stop)) = sub_entry.get_start_stop_times() {
                                let mut title = String::new();
                                if let Some(tags) = sub_entry.get_tags() {
                                    if let Some(tag) = tags.get::<gst::tags::Title>() {
                                        title = tag.get().unwrap().to_owned();
                                    };
                                };
                                info.chapters.push(Chapter::new(
                                    sub_entry.get_uid(),
                                    &title,
                                    Timestamp::from_signed_nano(start),
                                    Timestamp::from_signed_nano(stop),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
}
