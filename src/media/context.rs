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

use std::collections::HashSet;

use std::path::PathBuf;

use super::Timestamp;
use super::Chapter;


pub enum ContextMessage {
    VideoFrame,
    AudioFrame,
}

pub struct Context {
    ctx_tx: Sender<ContextMessage>,

    pub pipeline: gst::Pipeline,
    pub file_name: String,
    pub name: String,

    pub artist: String,
    pub title: String,
    pub duration: Timestamp,
    pub description: String,
    pub chapters: Vec<Chapter>,

    pub thumbnail: Option<Vec<u8>>,

    // TODO: use HashMaps for video_stream and audio_stream
    pub video_stream: Option<usize>,
    pub video_codec: String,
    //pub video_decoder: Option<ffmpeg::codec::decoder::Video>,

    pub audio_stream: Option<usize>,
    pub audio_codec: String,
    //pub audio_decoder: Option<ffmpeg::codec::decoder::Audio>,
}


macro_rules! assign_str_tag(
    ($target:expr, $tags:expr, $TagType:ty) => {
        if $target.is_empty() {
            match $tags.get::<$TagType>() {
                Some(tag) => $target = tag.get().unwrap().to_owned(),
                None => (),
            };
        }
    };
);


impl Context {
    pub fn new(
        path: &PathBuf,
        ctx_tx: Sender<ContextMessage>,
        video_area: &DrawingArea
    ) -> Result<Context, String>
    {
        println!("\n*** Attempting to open {:?}", path);

        let pipeline = gst::Pipeline::new(None);
        let dec = gst::ElementFactory::make("uridecodebin", "input").unwrap();
        let url = match Url::from_file_path(path.as_path()) {
            Ok(url) => url.into_string(),
            Err(_) => "Failed to convert path into URL".to_owned(),
        };
        dec.set_property("uri", &gst::Value::from(&url)).unwrap();
        pipeline.add(&dec).unwrap();

        let mut new_ctx = Context{
            ctx_tx: ctx_tx,
            pipeline: pipeline,
            file_name: String::from(path.file_name().unwrap().to_str().unwrap()),
            name: String::from(path.file_stem().unwrap().to_str().unwrap()),

            artist: String::new(),
            title: String::new(),
            duration: Timestamp::new(),
            description: String::new(),
            chapters: Vec::new(),

            thumbnail: None,

            video_stream: None,
            video_codec: String::new(),
            //video_decoder: None,

            audio_stream: None,
            audio_codec: String::new(),
            //audio_decoder: None,
        };

        let pipeline_clone = new_ctx.pipeline.clone();
        dec.connect_pad_added(move |dec_arg, src_pad| {
            if !src_pad.is_linked() {
                // TODO: build actual queues for audio and video with named elements
                // TODO: See if the drawingareas could be set after the initiatlization
                // TODO: See how to notify the context of a new audio / video stream
                // in a thread sage way
                let queue = gst::ElementFactory::make("queue", None).unwrap();
                let sink = gst::ElementFactory::make("fakesink", None).unwrap();
                let elements = &[&queue, &sink];
                pipeline_clone.add_many(elements).unwrap();
                gst::Element::link_many(elements).unwrap();
                for e in elements {
                    e.sync_state_with_parent().unwrap();
                }
                let sink_pad = queue.get_static_pad("sink").unwrap();
                assert_eq!(src_pad.link(&sink_pad), gst::PadLinkReturn::Ok);
            }
            println!("Pad added");
        });

        assert_ne!(
            new_ctx.pipeline.set_state(gst::State::Playing),
            gst::StateChangeReturn::Failure
        );

        let bus = new_ctx.pipeline.get_bus().unwrap();

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
                    assign_str_tag!(new_ctx.title, tags, Title);
                    assign_str_tag!(new_ctx.artist, tags, Artist);
                    assign_str_tag!(new_ctx.artist, tags, AlbumArtist);
                    assign_str_tag!(new_ctx.video_codec, tags, VideoCodec);
                    assign_str_tag!(new_ctx.audio_codec, tags, AudioCodec);

                    /*match tags.get::<PreviewImage>() {
                        // TODO: check if that happens, that would be handy for videos
                        Some(preview_tag) => println!("Found a PreviewImage tag"),
                        None => (),
                    };*/

                    match tags.get::<Image>() {
                        // TODO: distinguish front/back cover (take the first one?)
                        Some(image_tag) => {
                            match image_tag.get() {
                                Some(sample) => match sample.get_buffer() {
                                    Some(buffer) => (), // TODO: how do we get the buffer?
                                    None => (),
                                },
                                None => (),
                            };
                        },
                        None => (),
                    };
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
                                        new_ctx.duration = Timestamp::from_sec_time_factor(
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

        assert_ne!(
            new_ctx.pipeline.set_state(gst::State::Null),
            gst::StateChangeReturn::Failure
        );

        Ok(new_ctx)
    }
}
