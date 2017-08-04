extern crate gstreamer as gst;
use gstreamer::*;

extern crate glib;
use glib::ObjectExt;

extern crate url;
use url::Url;

use std::collections::HashMap;

use std::path::PathBuf;

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use super::{Chapter, Timestamp};

pub enum ContextMessage {
    FailedToOpenMedia,
    GotVideoWidget,
    //HaveAudioFrame,
    //HaveVideoFrame,
    HaveVideoWidget(glib::Value),
    OpenedMedia,
}

pub struct Context {
    pub pipeline: gst::Pipeline,

    pub path: PathBuf,
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
    fn new(path: PathBuf) -> Self {
        Context{
            pipeline: gst::Pipeline::new(None),

            file_name: String::from(path.file_name().unwrap().to_str().unwrap()),
            name: String::from(path.file_stem().unwrap().to_str().unwrap()),
            path: path,

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

    pub fn open_media_path(
        path: PathBuf,
        ctx_tx: Sender<ContextMessage>,
        ui_rx: Receiver<ContextMessage>,
    ) -> Result<Context, String>
    {
        println!("\nAttempting to open {:?}", path);

        let ctx = Context::new(path);
        ctx.build_pipeline(&ctx_tx, ui_rx);
        ctx.register_bus_inspector(ctx_tx);

        match ctx.play() {
            Ok(_) => Ok(ctx),
            Err(error) => Err(error),
        }
    }

    pub fn play(&self) -> Result<(), String> {
        let ret = self.pipeline.set_state(gst::State::Playing);
        if ret == gst::StateChangeReturn::Failure {
            return Err("could not set media in palying state".into());
        }
        Ok(())
    }

    pub fn stop(&self) {
        let ret = self.pipeline.set_state(gst::State::Null);
        if ret == gst::StateChangeReturn::Failure {
            println!("could not set media in Null state");
            //return Err("could not set media in Null state".into());
        }
    }

    // TODO: handle errors
    // Uses ctx_tx & ui_rx to allow the UI integrate the GstGtkWidget
    fn build_pipeline(&self,
        ctx_tx: &Sender<ContextMessage>,
        ui_rx: Receiver<ContextMessage>
    )
    {
        let dec = gst::ElementFactory::make("uridecodebin", "input").unwrap();
        let url = match Url::from_file_path(self.path.as_path()) {
            Ok(url) => url.into_string(),
            Err(_) => "Failed to convert path to URL".to_owned(),
        };
        dec.set_property("uri", &gst::Value::from(&url)).unwrap();
        self.pipeline.add(&dec).unwrap();

        let pipeline_clone = self.pipeline.clone();
        // Need an Arc Mutex on ctx_tx because Rust consider this method
        // as a candidate for being called by multiple threads
        let ctx_tx_arc_mtx = Arc::new(Mutex::new(ctx_tx.clone()));
        let ui_rx_arc_mtx = Arc::new(Mutex::new(ui_rx));
        dec.connect_pad_added(move |_, src_pad| {
            let ref pipeline = pipeline_clone;

            let caps = src_pad.get_current_caps().unwrap();
            let structure = caps.get_structure(0).unwrap();
            let name = structure.get_name();

            // TODO: build only one queue by stream type (audio / video)
            if name.starts_with("audio/") {
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
            } else if name.starts_with("video/") {
                let queue = gst::ElementFactory::make("queue", None).unwrap();
                let convert = gst::ElementFactory::make("videoconvert", None).unwrap();
                let scale = gst::ElementFactory::make("videoscale", None).unwrap();

                let (sink, widget_val) = if let Some(gtkglsink) = ElementFactory::make("gtkglsink", None) {
                    let glsinkbin = ElementFactory::make("glsinkbin", "video_sink").unwrap();
                    glsinkbin
                        .set_property("sink", &gtkglsink.to_value())
                        .unwrap();

                    let widget_val = gtkglsink.get_property("widget").unwrap();
                    (glsinkbin, widget_val)
                } else {
                    let sink = ElementFactory::make("gtksink", "video_sink").unwrap();
                    let widget_val = sink.get_property("widget").unwrap();
                    (sink, widget_val)
                };

                // Pass the video_sink for integration in the UI
                ctx_tx_arc_mtx.lock()
                    .expect("Failed to lock ctx_tx mutex, while building the video queue")
                    .send(ContextMessage::HaveVideoWidget(widget_val))
                    .expect("Failed to notify UI, while building the video queue");
                // Wait for the widget to be included, otherwise the pipeline
                // embeds it in a default window
                match ui_rx_arc_mtx.lock()
                    .expect("Failed to lock ui_rx mutex, while building the video queue")
                    .recv() {
                        Ok(message) => match message {
                            ContextMessage::GotVideoWidget => (),
                            _ => panic!("Unexpected message while waiting for GotVideoWidget"),
                        },
                        Err(error) => panic!("Error while waiting for GotVideoWidget: {:?}", error),
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
    }

    // Uses ctx_tx to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self, ctx_tx: Sender<ContextMessage>) {
        // TODO: restore media info processing
        let bus = self.pipeline.get_bus().unwrap();
        bus.add_watch(move |_, msg| {
            let mut keep_going = true;

            match msg.view() {
                MessageView::Eos(..) => {
                    ctx_tx.send(ContextMessage::OpenedMedia)
                        .expect("Failed to notify UI");
                    keep_going = false;
                },
                MessageView::Error(err) => {
                    println!(
                        "Error from {}: {} ({:?})",
                        msg.get_src().get_path_string(),
                        err.get_error(),
                        err.get_debug()
                    );
                    ctx_tx.send(ContextMessage::FailedToOpenMedia)
                        .expect("Failed to notify UI");
                    keep_going = false;
                },
                MessageView::AsyncDone(_) => {
                    ctx_tx.send(ContextMessage::OpenedMedia)
                        .expect("Failed to notify UI");
                    keep_going = false;
                },
                _ => (),
            };

            glib::Continue(keep_going)
        });
    }
}
