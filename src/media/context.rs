extern crate gstreamer as gst;
use gstreamer::*;

extern crate gtk;
use gtk::DrawingArea;

extern crate glib;
use glib::ObjectExt;
use glib::value::FromValueOptional;

extern crate url;
use url::Url;

use std::sync::mpsc::Sender;

use std::path::PathBuf;

use std::collections::HashMap;

use super::{Chapter, Timestamp};

pub enum ContextMessage {
    OpenedMedia(Context),
    FailedToOpenMedia,
    VideoFrame,
    AudioFrame,
}

pub struct Context {
    tx: Sender<ContextMessage>,
    pub pipeline: gst::Pipeline,

    pub file_name: String,
    pub name: String,

    pub artist: String,
    pub title: String,
    pub duration: Timestamp,
    pub description: String,
    pub chapters: Vec<Chapter>,

    pub thumbnail: Option<Vec<u8>>,

    pub video_streams: HashMap<String, gst::Caps>,
    pub video_best: Option<String>,
    pub video_codec: String,

    pub audio_streams: HashMap<String, gst::Caps>,
    pub audio_best: Option<String>,
    pub audio_codec: String,
}

macro_rules! assign_str_tag(
    ($target:expr, $tags:expr, $TagType:ty) => {
        if $target.is_empty() {
            if let Some(tag) = $tags.get::<$TagType>() {
                $target = tag.get().unwrap().to_owned();
            }
        }
    };
);


impl Context {
    fn new(ctx_tx: Sender<ContextMessage>) -> Context {
        Context{
            tx: ctx_tx,
            pipeline: gst::Pipeline::new(None),

            file_name: String::new(),
            name: String::new(),

            artist: String::new(),
            title: String::new(),
            duration: Timestamp::new(),
            description: String::new(),
            chapters: Vec::new(),

            thumbnail: None,

            video_streams: HashMap::new(),
            video_best: None,
            video_codec: String::new(),

            audio_streams: HashMap::new(),
            audio_best: None,
            audio_codec: String::new(),
        }
    }

    // result will be transmitted through ctx_tx
    pub fn open_media_path_thread(path: PathBuf, ctx_tx: Sender<ContextMessage>) {
        let mut ctx = Context::new(ctx_tx.clone());
        ctx.file_name = String::from(path.file_name().unwrap().to_str().unwrap());
        ctx.name = String::from(path.file_stem().unwrap().to_str().unwrap());

        println!("\n*** Attempting to open {:?}", path);

        let dec = gst::ElementFactory::make("uridecodebin", "input").unwrap();
        let url = match Url::from_file_path(path.as_path()) {
            Ok(url) => url.into_string(),
            Err(_) => "Failed to convert path into URL".to_owned(),
        };
        dec.set_property("uri", &gst::Value::from(&url)).unwrap();
        ctx.pipeline.add(&dec).unwrap();

        // TODO: use a channel to send the channels names and listen to it in the message loop below
        // or use an Arc + mutex?
        let pipeline_clone = ctx.pipeline.clone();
        dec.connect_pad_added(move |dec_arg, src_pad| {
            let ref pipeline = pipeline_clone;

            let caps = src_pad.get_current_caps().unwrap();
            let structure = caps.get_structure(0).unwrap();
            let name = structure.get_name();

            let (is_audio, is_video) = {
                (name.starts_with("audio/"), name.starts_with("video/"))
            };

            // TODO: build only one queue by stream type (audio / video)
            if is_audio {
                // TODO: add a probe to send audio frames through the ctx channel
                let queue = gst::ElementFactory::make("queue", None).unwrap();
                let convert = gst::ElementFactory::make("audioconvert", None).unwrap();
                let resample = gst::ElementFactory::make("audioresample", None).unwrap();
                let sink = gst::ElementFactory::make("autoaudiosink", "audio_sink").unwrap();

                let elements = &[&queue, &convert, &resample, &sink];
                pipeline.add_many(elements).unwrap();
                gst::Element::link_many(elements).unwrap();

                for e in elements {
                    e.sync_state_with_parent().unwrap();
                }

                let sink_pad = queue.get_static_pad("sink").unwrap();
                assert_eq!(src_pad.link(&sink_pad), gst::PadLinkReturn::Ok);
            } else if is_video {
                let queue = gst::ElementFactory::make("queue", None).unwrap();
                let convert = gst::ElementFactory::make("videoconvert", None).unwrap();
                let scale = gst::ElementFactory::make("videoscale", None).unwrap();
                let sink = if let Some(gtkglsink) = ElementFactory::make("gtkglsink", None) {
                    let glsinkbin = ElementFactory::make("glsinkbin", "video_sink").unwrap();
                    glsinkbin
                        .set_property("sink", &gtkglsink.to_value())
                        .unwrap();
                    glsinkbin
                } else {
                    let sink = ElementFactory::make("gtksink", "video_sink").unwrap();
                    sink
                };

                let elements = &[&queue, &convert, &scale, &sink];
                pipeline.add_many(elements).unwrap();
                gst::Element::link_many(elements).unwrap();

                for e in elements {
                    e.sync_state_with_parent().unwrap();
                }

                let sink_pad = queue.get_static_pad("sink").unwrap();
                assert_eq!(src_pad.link(&sink_pad), gst::PadLinkReturn::Ok);
            }
        });

        if ctx.pipeline.set_state(gst::State::Playing) == gst::StateChangeReturn::Failure {
            ctx_tx.send(ContextMessage::FailedToOpenMedia);
            return;
        }

        let bus = ctx.pipeline.get_bus().unwrap();

        loop {
            let msg = match bus.timed_pop(::std::u64::MAX) {
                None => break,
                Some(msg) => msg,
            };

            match msg.view() {
                MessageView::Eos(_) => break,
                MessageView::Error(err) => {
                    println!(
                        "\n** Error from {}: {} ({:?})",
                        msg.get_src().get_path_string(),
                        err.get_error(),
                        err.get_debug()
                    );
                    break;
                },
                MessageView::Tag(msg_tag) => {
                    let tags = msg_tag.get_tags();
                    assign_str_tag!(ctx.title, tags, Title);
                    assign_str_tag!(ctx.artist, tags, Artist);
                    assign_str_tag!(ctx.artist, tags, AlbumArtist);
                    assign_str_tag!(ctx.video_codec, tags, VideoCodec);
                    assign_str_tag!(ctx.audio_codec, tags, AudioCodec);

                    /*match tags.get::<PreviewImage>() {
                        // TODO: check if that happens, that would be handy for videos
                        Some(preview_tag) => println!("Found a PreviewImage tag"),
                        None => (),
                    };*/

                    // TODO: distinguish front/back cover (take the first one?)
                    if let Some(image_tag) = tags.get::<Image>() {
                        if let Some(sample) = image_tag.get() {
                            if let Some(buffer) = sample.get_buffer() {
                                if let Some(map) = buffer.map_read() {
                                    // TODO: build an aligned_image directly
                                    // so that we can save one copy
                                    // and implement a wrapper on an aligned_image
                                    // in image_surface
                                    let mut thumbnail = Vec::with_capacity(map.get_size());
                                    thumbnail.extend_from_slice(map.as_slice());
                                    ctx.thumbnail = Some(thumbnail);
                                }
                            }
                        }
                    }
                },
                MessageView::StreamStatus(status) => {
                    let name = msg.get_src().get_name();
                    let status = status.get();
                    let (status_type, element) = (status.0, status.1.unwrap());
                    println!("\nStream status: {:?} - {}", status_type, name);
                    // TODO: see who to handle multithreading in pad_added and
                    // make a decision about this (remove or use it to update ctx)
                    if true { // status_type == gst::StreamStatusType::Enter {
                        if true { //name.starts_with("src") {
                            println!("src pads");
                            match element.iterate_src_pads() {
                                Some(ref mut pad_iter) => {
                                    loop {
                                        match pad_iter.next() {
                                            Ok(pad) => {
                                                let pad: gst::Pad = pad.get().unwrap();
                                                match pad.get_stream_id() {
                                                    Some(id) => {
                                                        println!("\tstream id: {}", &id);
                                                    },
                                                    None => println!("\tno stream id"),
                                                }

                                                match pad.get_stream() {
                                                    Some(stream) => println!("\tstream: {:?}", &stream),
                                                    None => (),
                                                }

                                                match pad.get_current_caps() {
                                                    Some(caps) => {
                                                        println!("\tcaps: {:?}", caps);

                                                        for structure in caps.iter() {
                                                            let name = structure.get_name();
                                                            println!("\t\tstructure: {}", name);
                                                            //println!("\t\tstructure: {} - {:?}", name, structure);
                                                        }
                                                    },
                                                    None => println!("\tno caps"),
                                                };
                                            },
                                            Err(_) => break,
                                        }
                                    }
                                },
                                None => println!("\tempty pad iterator"),
                            };

                            println!("sink pads");
                            match element.iterate_sink_pads() {
                                Some(ref mut pad_iter) => {
                                    loop {
                                        match pad_iter.next() {
                                            Ok(pad) => {
                                                let pad: gst::Pad = pad.get().unwrap();
                                                match pad.get_stream_id() {
                                                    Some(id) => {
                                                        println!("\tstream id: {}", &id);
                                                    },
                                                    None => println!("\tno stream id"),
                                                }

                                                match pad.get_stream() {
                                                    Some(stream) => println!("\tstream: {:?}", &stream),
                                                    None => (),
                                                }

                                                match pad.get_current_caps() {
                                                    Some(caps) => {
                                                        println!("\tcaps: {:?}", caps);

                                                        for structure in caps.iter() {
                                                            let name = structure.get_name();
                                                            println!("\t\tstructure: {} - {:?}", name, structure);
                                                        }
                                                    },
                                                    None => println!("\tno caps"),
                                                };
                                            },
                                            Err(_) => break,
                                        }
                                    }
                                },
                                None => println!("\tempty pad iterator"),
                            };

                            // TODO: fix duration determination
                            // there must be some better way
                            // Note: how is the info encoded for a multiple chapter media?
                            if name == "src" {
                                match element.query_duration(gst::Format::Time) {
                                    Some(duration) => {
                                        ctx.duration = Timestamp::from_sec_time_factor(
                                            duration, 1f64 / 1_000_000_000f64
                                        );
                                    },
                                    None => (),
                                };
                            }
                        }
                    }
                },
                MessageView::AsyncDone(_) => break,
                _ => (),
            }
        }

        // TODO: be more precise in order to distinguish with error playing
        if ctx.pipeline.set_state(gst::State::Null) == gst::StateChangeReturn::Failure {
            ctx_tx.send(ContextMessage::FailedToOpenMedia);
            return;
        }

        ctx_tx.send(ContextMessage::OpenedMedia(ctx));
    }
}
