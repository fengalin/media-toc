extern crate gstreamer as gst;
use gstreamer::*;

extern crate gtk;
use gtk::{BoxExt, ContainerExt, WidgetExt};

extern crate glib;
use glib::ObjectExt;

extern crate url;
use url::Url;

use std::collections::HashMap;

use std::path::PathBuf;

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use std::thread;

use super::{Chapter, Timestamp};

pub enum ContextMessage {
    OpenedMedia,
    FailedToOpenMedia,
    VideoFrame,
    AudioFrame,
}

pub struct Context {
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
    fn new() -> Self {
        Context{
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

    // will add a GstGtkWidget to the video_box
    fn prepare_video_sink(&self, video_box: &gtk::Box) -> gst::Element {
        let (sink, widget_val) = if let Some(gtkglsink) = ElementFactory::make("gtkglsink", None) {
            let glsinkbin = ElementFactory::make("glsinkbin", None).unwrap();
            glsinkbin
                .set_property("sink", &gtkglsink.to_value())
                .unwrap();

            let widget_val = gtkglsink.get_property("widget").unwrap();
            (glsinkbin, widget_val)
        } else {
            let sink = ElementFactory::make("gtksink", None).unwrap();
            let widget_val = sink.get_property("widget").unwrap();
            (sink, widget_val)
        };

        for child in video_box.get_children() {
            video_box.remove(&child);
        }

        let widget = widget_val.get::<gtk::Widget>().unwrap();
        video_box.pack_start(&widget, true, true, 0);

        video_box.show_all();

        sink
    }

    pub fn open_media_path(
        path: PathBuf,
        video_box: &gtk::Box,
        ctx_tx: Sender<ContextMessage>,
    ) -> Result<Context, String>
    {
        let mut ctx = Context::new();
        ctx.file_name = String::from(path.file_name().unwrap().to_str().unwrap());
        ctx.name = String::from(path.file_stem().unwrap().to_str().unwrap());

        println!("\n*** Attempting to open {:?}", path);
        // prepare pipeline
        let dec = gst::ElementFactory::make("uridecodebin", "input").unwrap();
        let url = match Url::from_file_path(path.as_path()) {
            Ok(url) => url.into_string(),
            Err(_) => "Failed to convert path into URL".to_owned(),
        };
        dec.set_property("uri", &gst::Value::from(&url)).unwrap();
        ctx.pipeline.add(&dec).unwrap();

        // prepare the video sink while we are in the main (GTK) thread
        let video_sink = ctx.prepare_video_sink(video_box);

        let pipeline_clone = ctx.pipeline.clone();
        dec.connect_pad_added(move |_, src_pad| {
            let ref pipeline = pipeline_clone;

            let caps = src_pad.get_current_caps().unwrap();
            let structure = caps.get_structure(0).unwrap();
            let name = structure.get_name();

            let (is_audio, is_video) = {
                (name.starts_with("audio/"), name.starts_with("video/"))
            };

            // TODO: build only one queue by stream type (audio / video)
            if is_audio {
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

                let elements = &[&queue, &convert, &scale, &video_sink];
                pipeline.add_many(elements).unwrap();
                gst::Element::link_many(elements).unwrap();

                for e in elements {
                    e.sync_state_with_parent().unwrap();
                }

                let sink_pad = queue.get_static_pad("sink").unwrap();
                assert_eq!(src_pad.link(&sink_pad), gst::PadLinkReturn::Ok);
            }
        });

        let bus = ctx.pipeline.get_bus().unwrap();

        bus.add_watch(move |_, msg| {
            let mut keep_going = true;

            match msg.view() {
                MessageView::Eos(..) => {
                    ctx_tx.send(ContextMessage::OpenedMedia);
                    keep_going = false;
                },
                MessageView::Error(err) => {
                    println!(
                        "Error from {}: {} ({:?})",
                        msg.get_src().get_path_string(),
                        err.get_error(),
                        err.get_debug()
                    );
                    ctx_tx.send(ContextMessage::FailedToOpenMedia);
                    keep_going = false;
                },
                MessageView::AsyncDone(_) => {
                    ctx_tx.send(ContextMessage::OpenedMedia);
                    keep_going = true;
                },
                _ => (),
            };

            glib::Continue(keep_going)
        });

        Ok(ctx)
    }

    pub fn play(&self) {
        let ret = self.pipeline.set_state(gst::State::Playing);
        if ret == gst::StateChangeReturn::Failure {
            println!("could not set media in palying state");
            //return Err("could not set media in palying state".into());
        }
        println!("Playing...");
    }

    pub fn stop(&self) {
        let ret = self.pipeline.set_state(gst::State::Null);
        if ret == gst::StateChangeReturn::Failure {
            println!("could not set media in Null state");
            //return Err("could not set media in Null state".into());
        }
    }
}
