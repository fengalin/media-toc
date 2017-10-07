extern crate gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::{BinExt, Caps, ElementFactory, GstObjectExt, PadExt, QueryView,
                TocScope, TocEntryType};

extern crate gstreamer_audio as gst_audio;
extern crate gstreamer_app as gst_app;

extern crate glib;
use glib::{Cast, ObjectExt, ToValue};

extern crate gtk;
use gtk::{BoxExt, ContainerExt};

use url::Url;

use std::path::PathBuf;

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use std::i32;

use super::{AlignedImage, DoubleAudioBuffer, Chapter, MediaInfo, Timestamp};

macro_rules! assign_str_tag(
    ($target:expr, $tags:expr, $TagType:ty) => {
        if $target.is_empty() {
            if let Some(tag) = $tags.get::<$TagType>() {
                $target = tag.get().unwrap().to_owned();
            }
        }
    };
);

pub enum ContextMessage {
    AsyncDone,
    Eos,
    FailedToOpenMedia,
    InitDone,
}

pub struct Context {
    pipeline: gst::Pipeline,
    position_element: Option<gst::Element>,
    position_query: gst::Query,

    pub path: PathBuf,
    pub file_name: String,
    pub name: String,

    pub info: Arc<Mutex<MediaInfo>>,
}

// FIXME: need to `release_request_pad` on the tee
// maybe this should be done in a `drop`. At least, it
// should be done before the pipeline is reconstructed
impl Context {
    pub fn new(
        path: PathBuf,
        dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
        video_widget_box: gtk::Box,
        ctx_tx: Sender<ContextMessage>,
    ) -> Result<Context, String>
    {
        println!("\n\n* Attempting to open {:?}", path);

        let mut ctx = Context {
            pipeline: gst::Pipeline::new("pipeline"),
            position_element: None,
            position_query: gst::Query::new_position(gst::Format::Time),

            file_name: String::from(path.file_name().unwrap().to_str().unwrap()),
            name: String::from(path.file_stem().unwrap().to_str().unwrap()),
            path: path,

            info: Arc::new(Mutex::new(MediaInfo::new())),
        };

        ctx.build_pipeline(dbl_audio_buffer_mtx, video_widget_box);

        ctx.register_bus_inspector(ctx_tx);

        match ctx.pause() {
            Ok(_) => Ok(ctx),
            Err(error) => Err(error),
        }
    }

    pub fn get_position(&mut self) -> u64 {
        let pipeline = self.pipeline.clone();
        self.position_element.get_or_insert_with(|| {
            if let Some(video) = pipeline.get_by_name("video_sink") {
                video
            } else if let Some(audio) = pipeline.get_by_name("audio_playback_sink") {
                audio
            } else {
                panic!("No sink in pipeline");
            }
        }).query(self.position_query.get_mut().unwrap());
        match self.position_query.view() {
            QueryView::Position(ref position) => position.get().1 as u64,
            _ => unreachable!(),
        }
    }

    pub fn get_duration(&self) -> u64 {
        match self.pipeline.query_duration(gst::Format::Time) {
            Some(duration) => if duration.is_positive() { duration as u64 } else { 0 },
            None => 0,
        }
    }

    pub fn get_state(&self) -> gst::State {
        let (_, current, _) = self.pipeline.get_state(10_000_000);
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
            if accurate { gst::SeekFlags::ACCURATE }
            else { gst::SeekFlags::KEY_UNIT };
        self.pipeline.seek_simple(gst::Format::Time, flags, position as i64)
            .ok().unwrap();
    }

    // TODO: handle errors
    fn build_pipeline(&mut self,
        dbl_audio_buffer_mtx: Arc<Mutex<DoubleAudioBuffer>>,
        video_widget_box: gtk::Box,
    ) {
        let src = gst::ElementFactory::make("uridecodebin", "input").unwrap();
        let url = match Url::from_file_path(self.path.as_path()) {
            Ok(url) => url.into_string(),
            Err(_) => "Failed to convert path to URL".to_owned(),
        };
        src.set_property("uri", &gst::Value::from(&url)).unwrap();
        self.pipeline.add(&src).unwrap();

        // audio sink init
        let audio_sink = gst::ElementFactory::make("autoaudiosink", "audio_playback_sink").unwrap();

        // video sink init
        let (video_sink, widget_val) = if let Some(gtkglsink) = ElementFactory::make("gtkglsink", None) {
            let glsinkbin = ElementFactory::make("glsinkbin", "video_sink").unwrap();
            glsinkbin.set_property("sink", &gtkglsink.to_value()).unwrap();
            let widget_val = gtkglsink.get_property("widget").unwrap();
            (glsinkbin, widget_val)
        } else {
            let sink = ElementFactory::make("gtksink", "video_sink").unwrap();
            let widget_val = sink.get_property("widget").unwrap();
            (sink, widget_val)
        };
        for child in video_widget_box.get_children() {
            video_widget_box.remove(&child);
        }
        let widget = widget_val.get::<gtk::Widget>()
            .expect("Failed to get GstGtkWidget glib::Value as gtk::Widget");
        video_widget_box.pack_start(&widget, true, true, 0);

        // Prepare pad configuration callback
        let pipeline_clone = self.pipeline.clone();
        let info_arc_mtx = Arc::clone(&self.info);
        src.connect_pad_added(move |_, src_pad| {
            let pipeline = &pipeline_clone;

            let caps = src_pad.get_current_caps().unwrap();
            let structure = caps.get_structure(0).unwrap();
            let name = structure.get_name();

            // TODO: build only one queue by stream type (audio / video)
            if name.starts_with("audio/") {
                let is_first = {
                    let info = &mut info_arc_mtx.lock()
                        .expect("Failed to lock media info while initializing audio stream");
                    info.audio_streams.insert(name.to_owned(), caps.clone());
                    let is_first = info.audio_best.is_none();
                    info.audio_best.get_or_insert(name.to_owned());

                    is_first
                };

                if is_first {
                    Context::build_audio_pipeline(
                        pipeline, src_pad, &audio_sink, dbl_audio_buffer_mtx.clone()
                    );
                }
            } else if name.starts_with("video/") {
                let is_first = {
                    let info = &mut info_arc_mtx.lock()
                        .expect("Failed to lock media info while initializing audio stream");
                    info.video_streams.insert(name.to_owned(), caps.clone());
                    let is_first = info.video_best.is_none();
                    info.video_best.get_or_insert(name.to_owned());

                    is_first
                };

                if is_first {
                    Context::build_video_pipeline(pipeline, src_pad, &video_sink);
                }
            }
        });
    }

    fn build_audio_pipeline (
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
            &playback_queue, &playback_convert, &playback_resample, &audio_sink,
        ];

        let visu_queue = gst::ElementFactory::make("queue", "visu_queue").unwrap();

        // This is the buffer duration that will be queued in paused mode
        // TODO: make this configurable
        let queue_duration = 5_000_000_000u64; // 5s
        playback_queue.set_property("max-size-time", &gst::Value::from(&queue_duration)).unwrap();
        visu_queue.set_property("max-size-time", &gst::Value::from(&queue_duration)).unwrap();

        #[cfg(feature = "profiling-audio-queue")]
        visu_queue.connect("overrun", false, |_| {
            println!("Audio visu queue OVERRUN");
            None
        }).ok().unwrap();

        let visu_convert = gst::ElementFactory::make("audioconvert", None).unwrap();
        let visu_sink = gst::ElementFactory::make("appsink", "audio_visu_sink").unwrap();
        let visu_sink_pad = visu_queue.get_static_pad("sink").unwrap();

        {
            let visu_elements = &[&visu_queue, &visu_convert, &visu_sink];
            let tee = gst::ElementFactory::make("tee", "audio_tee").unwrap();
            let mut elements = vec!(&tee);
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
            assert_eq!(tee_playback_src_pad.link(&playback_sink_pad), gst::PadLinkReturn::Ok);

            let tee_visu_src_pad = tee.get_request_pad("src_%u").unwrap();
            assert_eq!(tee_visu_src_pad.link(&visu_sink_pad), gst::PadLinkReturn::Ok);

            for e in elements { e.sync_state_with_parent().unwrap(); }
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
        appsink.set_property("sync", &gst::Value::from(&false)).unwrap();
        // and don't block pipeline when switching state
        appsink.set_property("async", &gst::Value::from(&false)).unwrap();

        {
            dbl_audio_buffer_mtx.lock()
                .expect("Context::build_audio_pipeline: couldn't lock dbl_audio_buffer_mtx")
                .set_audio_caps_and_ref(
                    &src_pad.get_current_caps().unwrap(),
                    &audio_sink
                );
        }

        let dbl_audio_buffer_mtx_eos = Arc::clone(&dbl_audio_buffer_mtx);
        appsink.set_callbacks(gst_app::AppSinkCallbacks::new(
            /* eos */
            move |_| {
                dbl_audio_buffer_mtx_eos.lock()
                    .expect("appsink: eos: couldn't lock dbl_audio_buffer_mtx")
                    .handle_eos();
            },
            /* new_preroll */
            |_| gst::FlowReturn::Ok,
            /* new_samples */
            move |appsink| {
                match appsink.pull_sample() {
                    Some(sample) => {
                        {
                            dbl_audio_buffer_mtx.lock()
                                .expect("appsink: new_samples: couldn't lock dbl_audio_buffer_mtx")
                                .push_gst_sample(sample);
                        }
                        gst::FlowReturn::Ok
                    },
                    None => gst::FlowReturn::Eos,
                }
            },
        ));
    }

    fn build_video_pipeline(
        pipeline: &gst::Pipeline,
        src_pad: &gst::Pad,
        video_sink: &gst::Element
    ) {
        let queue = gst::ElementFactory::make("queue", None).unwrap();
        let convert = gst::ElementFactory::make("videoconvert", None).unwrap();
        let scale = gst::ElementFactory::make("videoscale", None).unwrap();

        let elements = &[&queue, &convert, &scale, &video_sink];
        pipeline.add_many(elements).unwrap();
        gst::Element::link_many(elements).unwrap();

        for e in elements { e.sync_state_with_parent().unwrap(); }

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
                    ctx_tx.send(ContextMessage::Eos)
                        .expect("Failed to notify UI");
                    return glib::Continue(false);
                },
                gst::MessageView::Error(err) => {
                    eprintln!("Error from {}: {} ({:?})",
                        msg.get_src().get_path_string(),
                        err.get_error(), err.get_debug()
                    );
                    ctx_tx.send(ContextMessage::FailedToOpenMedia)
                        .expect("Failed to notify UI");
                    return glib::Continue(false);
                },
                gst::MessageView::AsyncDone(_) => {
                    if !init_done {
                        init_done = true;
                        ctx_tx.send(ContextMessage::InitDone)
                            .expect("Failed to notify UI");
                    } else {
                        ctx_tx.send(ContextMessage::AsyncDone)
                            .expect("Failed to notify UI");
                    }
                },
                gst::MessageView::Tag(msg_tag) => {
                    if !init_done {
                        Context::add_tags(msg_tag.get_tags(), &info_arc_mtx);
                    }
                },
                gst::MessageView::Toc(msg_toc) => {
                    if !init_done {
                        let (toc, _) = msg_toc.get_toc();
                        if toc.get_scope() == TocScope::Global {
                            Context::add_toc(toc, &info_arc_mtx);
                        } else {
                            println!("Warning: Skipping toc with scope: {:?}", toc.get_scope());
                        }
                    }
                },
                _ => (),
            }

            glib::Continue(true)
        });
    }

    fn add_tags(tags: gst::TagList, info_arc_mtx: &Arc<Mutex<MediaInfo>>) {
        let info = &mut info_arc_mtx.lock()
            .expect("Failed to lock media info while reading tag data");
        assign_str_tag!(info.title, tags, gst::tags::Title);
        assign_str_tag!(info.artist, tags, gst::tags::Artist);
        assign_str_tag!(info.artist, tags, gst::tags::AlbumArtist);
        assign_str_tag!(info.container, tags, gst::tags::ContainerFormat);
        assign_str_tag!(info.video_codec, tags, gst::tags::VideoCodec);
        assign_str_tag!(info.audio_codec, tags, gst::tags::AudioCodec);

        // TODO: distinguish front/back cover (take the first one?)
        if let Some(image_tag) = tags.get::<gst::tags::Image>() {
            if let Some(sample) = image_tag.get() {
                if let Some(buffer) = sample.get_buffer() {
                    if let Some(map) = buffer.map_readable() {
                        info.thumbnail = AlignedImage::from_uknown_buffer(
                                map.as_slice()
                            ).ok();
                    }
                }
            }
        }
    }

    fn add_toc(toc: gst::Toc, info_arc_mtx: &Arc<Mutex<MediaInfo>>) {
        let info = &mut info_arc_mtx.lock()
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
                                    Timestamp::from_signed_nano(stop)
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
