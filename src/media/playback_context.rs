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

use toc;
use toc::{Chapter, Timestamp};

use super::{AlignedImage, ContextMessage, DoubleAudioBuffer, MediaInfo};

macro_rules! assign_tag(
    ($metadata_map:expr, $name:expr, $tags:expr, $TagType:ty) => {
        if let Some(tag) = $tags.get::<$TagType>() {
            $metadata_map.entry($name.to_owned()).or_insert_with(move || {
                tag.get().unwrap().to_owned()
            });
        }
    };
);

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

pub struct PlaybackContext {
    pipeline: gst::Pipeline,
    position_element: Option<gst::Element>,
    position_query: gst::Query,

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
        println!("\n\n* Opening {:?}...", path);

        let file_name = String::from(path.file_name().unwrap().to_str().unwrap());

        let mut this = PlaybackContext {
            pipeline: gst::Pipeline::new("pipeline"),
            position_element: None,
            position_query: gst::Query::new_position(gst::Format::Time),

            file_name: file_name.clone(),
            name: String::from(path.file_stem().unwrap().to_str().unwrap()),
            path: path,

            info: Arc::new(Mutex::new(MediaInfo::new())),
        };

        this.info
            .lock()
            .expect("PlaybackContext::new failed to lock media info")
            .metadata
            .insert(toc::METADATA_FILE_NAME.to_owned(), file_name);

        this.build_pipeline(dbl_audio_buffer_mtx, (*VIDEO_SINK).clone());
        this.register_bus_inspector(ctx_tx);

        match this.pause() {
            Ok(_) => Ok(this),
            Err(error) => Err(error),
        }
    }

    pub fn get_video_widget() -> gtk::Widget {
        let widget_val = (*VIDEO_SINK).get_property("widget").unwrap();
        widget_val.get::<gtk::Widget>().expect(
            "Failed to get GstGtkWidget glib::Value as gtk::Widget",
        )
    }

    pub fn get_position(&mut self) -> u64 {
        let pipeline = self.pipeline.clone();
        self.position_element
            .get_or_insert_with(|| if let Some(video) = pipeline.get_by_name("video_sink") {
                video
            } else if let Some(audio) = pipeline.get_by_name("audio_playback_sink") {
                audio
            } else {
                panic!("No sink in pipeline");
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
            .unwrap_or(0.into())
            .nanoseconds()
            .unwrap()
    }

    pub fn get_state(&self) -> gst::State {
        let (_, current, _) = self.pipeline.get_state(ClockTime::from(10_000_000));
        current
    }

    pub fn play(&self) -> Result<(), String> {
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
        let flags = gst::SeekFlags::FLUSH |
            if accurate {
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
        self.pipeline.seek(
                1f64,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                ClockTime::from(start),
                gst::SeekType::Set,
                ClockTime::from(end),
            )
            .ok()
            .unwrap();
    }

    // TODO: handle errors
    fn build_pipeline(
        &mut self,
        dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
        video_sink: gst::Element,
    ) {
        let file_src = gst::ElementFactory::make("filesrc", None).unwrap();
        file_src
            .set_property("location", &gst::Value::from(self.path.to_str().unwrap()))
            .unwrap();

        let decodebin = gst::ElementFactory::make("decodebin", None).unwrap();

        self.pipeline.add_many(&[&file_src, &decodebin]).unwrap();
        file_src.link(&decodebin).unwrap();

        let audio_sink = gst::ElementFactory::make("autoaudiosink", "audio_playback_sink").unwrap();

        // Prepare pad configuration callback
        let pipeline_clone = self.pipeline.clone();
        let info_arc_mtx = Arc::clone(&self.info);
        decodebin.connect_pad_added(move |_, src_pad| {
            let pipeline = &pipeline_clone;

            let caps = src_pad.get_current_caps().unwrap();
            let structure = caps.get_structure(0).unwrap();
            let name = structure.get_name();

            // TODO: build only one queue by stream type (audio / video)
            if name.starts_with("audio/") {
                let is_first = {
                    let info = &mut info_arc_mtx.lock().expect(
                        "Failed to lock media info while initializing audio stream",
                    );
                    info.audio_streams.insert(name.to_owned(), caps.clone());
                    let is_first = info.audio_best.is_none();
                    info.audio_best.get_or_insert(name.to_owned());

                    is_first
                };

                if is_first {
                    PlaybackContext::build_audio_queue(
                        pipeline,
                        src_pad,
                        &audio_sink,
                        Arc::clone(&dbl_audio_buffer_mtx),
                    );
                }
            } else if name.starts_with("video/") {
                let is_first = {
                    let info = &mut info_arc_mtx.lock().expect(
                        "Failed to lock media info while initializing audio stream",
                    );
                    info.video_streams.insert(name.to_owned(), caps.clone());
                    let is_first = info.video_best.is_none();
                    info.video_best.get_or_insert(name.to_owned());

                    is_first
                };

                if is_first {
                    PlaybackContext::build_video_queue(pipeline, src_pad, &video_sink);
                }
            }
        });
    }

    fn build_audio_queue(
        pipeline: &gst::Pipeline,
        src_pad: &gst::Pad,
        audio_sink: &gst::Element,
        dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
    ) {
        let playback_queue = gst::ElementFactory::make("queue", "playback_queue").unwrap();

        let playback_convert = gst::ElementFactory::make("audioconvert", None).unwrap();
        let playback_resample = gst::ElementFactory::make("audioresample", None).unwrap();
        let playback_sink_pad = playback_queue.get_static_pad("sink").unwrap();
        let playback_elements = &[
            &playback_queue,
            &playback_convert,
            &playback_resample,
            audio_sink,
        ];

        let visu_queue = gst::ElementFactory::make("queue", "visu_queue").unwrap();

        // This is the buffer duration that will be queued in paused mode
        // TODO: make this configurable
        let queue_duration = 5_000_000_000u64; // 5s
        playback_queue
            .set_property("max-size-time", &gst::Value::from(&queue_duration))
            .unwrap();
        visu_queue
            .set_property("max-size-time", &gst::Value::from(&queue_duration))
            .unwrap();

        #[cfg(feature = "trace-audio-queue")]
        visu_queue
            .connect("overrun", false, |_| {
                println!("Audio visu queue OVERRUN");
                None
            })
            .ok()
            .unwrap();

        let visu_convert = gst::ElementFactory::make("audioconvert", None).unwrap();
        let visu_sink = gst::ElementFactory::make("appsink", "audio_visu_sink").unwrap();
        let visu_sink_pad = visu_queue.get_static_pad("sink").unwrap();

        {
            let visu_elements = &[&visu_queue, &visu_convert, &visu_sink];
            let tee = gst::ElementFactory::make("tee", "audio_tee").unwrap();
            let mut elements = vec![&tee];
            elements.extend_from_slice(playback_elements);
            elements.extend_from_slice(visu_elements);
            pipeline.add_many(elements.as_slice()).unwrap();

            gst::Element::link_many(playback_elements).unwrap();
            gst::Element::link_many(visu_elements).unwrap();

            let tee_sink = tee.get_static_pad("sink").unwrap();
            assert_eq!(src_pad.link(&tee_sink), gst::PadLinkReturn::Ok);

            // TODO: in C, requested pads must be released when done
            // check if this also applies to the Rust binding
            let tee_playback_src_pad = tee.get_request_pad("src_%u").unwrap();
            assert_eq!(
                tee_playback_src_pad.link(&playback_sink_pad),
                gst::PadLinkReturn::Ok
            );

            let tee_visu_src_pad = tee.get_request_pad("src_%u").unwrap();
            assert_eq!(
                tee_visu_src_pad.link(&visu_sink_pad),
                gst::PadLinkReturn::Ok
            );

            for e in elements {
                e.sync_state_with_parent().unwrap();
            }
        }

        let appsink = visu_sink.dynamic_cast::<gst_app::AppSink>().unwrap();
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
                .expect(
                    "PlaybackContext::build_audio_pipeline: couldn't lock dbl_audio_buffer_mtx",
                )
                .set_audio_caps_and_ref(&src_pad.get_current_caps().unwrap(), audio_sink);
        }

        #[cfg(feature = "trace-audio-caps")]
        visu_sink_pad.connect_property_caps_notify(|pad| {
            println!("{:?}", pad.get_property_caps());
        });

        let dbl_audio_buffer_mtx_eos = Arc::clone(&dbl_audio_buffer_mtx);
        appsink.set_callbacks(
            gst_app::AppSinkCallbacksBuilder::new()
                .eos(move |_| {
                    dbl_audio_buffer_mtx_eos
                        .lock()
                        .expect("appsink: eos: couldn't lock dbl_audio_buffer")
                        .handle_eos();
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

    fn build_video_queue(pipeline: &gst::Pipeline, src_pad: &gst::Pad, video_sink: &gst::Element) {
        let queue = gst::ElementFactory::make("queue", None).unwrap();
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
        let info_arc_mtx = Arc::clone(&self.info);
        let mut init_done = false;
        self.pipeline.get_bus().unwrap().add_watch(move |_, msg| {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    ctx_tx.send(ContextMessage::Eos).expect(
                        "Failed to notify UI",
                    );
                }
                gst::MessageView::Error(err) => {
                    eprintln!(
                        "Error from {}: {} ({:?})",
                        msg.get_src().map(|s| s.get_path_string()).unwrap_or_else(
                            || {
                                String::from("None")
                            },
                        ),
                        err.get_error(),
                        err.get_debug()
                    );
                    ctx_tx.send(ContextMessage::FailedToOpenMedia).expect(
                        "Failed to notify UI",
                    );
                    return glib::Continue(false);
                }
                gst::MessageView::AsyncDone(_) => {
                    if !init_done {
                        init_done = true;
                        ctx_tx.send(ContextMessage::InitDone).expect(
                            "Failed to notify UI",
                        );
                    } else {
                        ctx_tx.send(ContextMessage::AsyncDone).expect(
                            "Failed to notify UI",
                        );
                    }
                }
                gst::MessageView::Tag(msg_tag) => {
                    if !init_done {
                        PlaybackContext::add_tags(&msg_tag.get_tags(), &info_arc_mtx);
                    }
                }
                gst::MessageView::Toc(msg_toc) => {
                    if !init_done {
                        let (toc, _) = msg_toc.get_toc();
                        if toc.get_scope() == TocScope::Global {
                            PlaybackContext::add_toc(&toc, &info_arc_mtx);
                        } else {
                            println!("Warning: Skipping toc with scope: {:?}", toc.get_scope());
                        }
                    }
                }
                _ => (),
            }

            glib::Continue(true)
        });
    }

    fn add_tags(tags: &gst::TagList, info_arc_mtx: &Arc<Mutex<MediaInfo>>) {
        let info = &mut info_arc_mtx.lock().expect(
            "Failed to lock media info while reading tag data",
        );
        assign_tag!(info.metadata, toc::METADATA_TITLE, tags, gst::tags::Title);
        assign_tag!(info.metadata, toc::METADATA_TITLE, tags, gst::tags::Album);
        assign_tag!(info.metadata, toc::METADATA_ARTIST, tags, gst::tags::Artist);
        assign_tag!(
            info.metadata,
            toc::METADATA_ARTIST,
            tags,
            gst::tags::AlbumArtist
        );

        assign_tag!(
            info.metadata,
            toc::METADATA_AUDIO_CODEC,
            tags,
            gst::tags::AudioCodec
        );
        assign_tag!(
            info.metadata,
            toc::METADATA_VIDEO_CODEC,
            tags,
            gst::tags::VideoCodec
        );
        assign_tag!(
            info.metadata,
            toc::METADATA_CONTAINER,
            tags,
            gst::tags::ContainerFormat
        );

        // TODO: distinguish front/back cover (take the first one?)
        if let Some(image_tag) = tags.get::<gst::tags::Image>() {
            if let Some(sample) = image_tag.get() {
                if let Some(buffer) = sample.get_buffer() {
                    if let Some(map) = buffer.map_readable() {
                        info.thumbnail = AlignedImage::from_uknown_buffer(map.as_slice()).ok();
                    }
                }
            }
        }
    }

    fn add_toc(toc: &gst::Toc, info_arc_mtx: &Arc<Mutex<MediaInfo>>) {
        let info = &mut info_arc_mtx.lock().expect(
            "Failed to lock media info while reading toc data",
        );
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
                        } /*else {
                            println!("Warning: Skipping toc sub entry with entry type: {:?}",
                                sub_entry.get_entry_type()
                            );
                        }*/
                    }
                } /*else {
                    println!("Warning: Skipping toc entry with entry type: {:?}",
                        entry.get_entry_type()
                    );
                }*/
            }
        }
    }
}
