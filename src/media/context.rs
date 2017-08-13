extern crate gstreamer as gst;
use gstreamer::{BinExt, BinExtManual, ElementExt, ElementFactory, GstObjectExt,
                PadExt, PadExtManual, TocScope, TocEntryType};

extern crate glib;
use glib::{ObjectExt, ToValue};

extern crate url;
use url::Url;

use std::path::PathBuf;

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use super::{AlignedImage, AudioBuffer, AudioCaps, Chapter, MediaInfo, Timestamp};

pub enum ContextMessage {
    AsyncDone,
    Eos,
    FailedToOpenMedia,
    GotVideoWidget,
    HaveAudioBuffer(AudioBuffer),
    HaveVideoWidget(glib::Value),
    InitDone,
}

pub struct Context {
    pub pipeline: gst::Pipeline,
    pub state: gst::State,

    pub path: PathBuf,
    pub file_name: String,
    pub name: String,

    pub info: Arc<Mutex<MediaInfo>>,
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
            pipeline: gst::Pipeline::new("pipeline"),
            state: gst::State::Null,

            file_name: String::from(path.file_name().unwrap().to_str().unwrap()),
            name: String::from(path.file_stem().unwrap().to_str().unwrap()),
            path: path,

            info: Arc::new(Mutex::new(MediaInfo::new())),
        }
    }

    pub fn open_media_path(
        path: PathBuf,
        ctx_tx: Sender<ContextMessage>,
        ui_rx: Receiver<ContextMessage>,
    ) -> Result<Context, String>
    {
        println!("\n\n* Attempting to open {:?}", path);

        let mut ctx = Context::new(path);
        ctx.build_pipeline(&ctx_tx, ui_rx);
        ctx.register_bus_inspector(ctx_tx);

        match ctx.pause() {
            Ok(_) => Ok(ctx),
            Err(error) => Err(error),
        }
    }

    pub fn get_position(&self) -> i64 {
        match self.pipeline.query_position(gst::Format::Time) {
            Some(duration) => duration,
            None => 0,
        }
    }

    pub fn get_duration(&self) -> Timestamp {
        match self.pipeline.query_duration(gst::Format::Time) {
            Some(duration) => Timestamp::from_signed_nano(duration),
            None => Timestamp::new(),
        }
    }

    pub fn play_pause(&mut self) -> Result<gst::State, String> {
        let (_, current, _) = self.pipeline.get_state(10_000_000);
        match current {
            gst::State::Playing => self.pause(),
            _ => self.play(),
        }
    }

    pub fn play(&mut self) -> Result<gst::State, String> {
        if self.pipeline.set_state(gst::State::Playing) == gst::StateChangeReturn::Failure {
            return Err("Could not set media in palying state".into());
        }
        self.state = gst::State::Playing;
        Ok(self.state)
    }

    pub fn pause(&mut self) -> Result<gst::State, String> {
        if self.pipeline.set_state(gst::State::Paused) == gst::StateChangeReturn::Failure {
            println!("could not set media in Paused state");
            return Err("Could not set media in Paused state".into());
        }
        self.state = gst::State::Paused;
        Ok(self.state)
    }

    pub fn stop(&mut self) {
        if self.pipeline.set_state(gst::State::Null) == gst::StateChangeReturn::Failure {
            println!("Could not set media in Null state");
            //return Err("could not set media in Null state".into());
        }
        self.state = gst::State::Null;
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
        let ctx_tx_arc_mtx = Arc::new(Mutex::new(ctx_tx.clone()));
        let ui_rx_arc_mtx = Arc::new(Mutex::new(ui_rx));
        dec.connect_pad_added(move |_, src_pad| {
            let pipeline = &pipeline_clone;

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

                for e in elements { e.sync_state_with_parent().unwrap(); }

                let sink_pad = queue.get_static_pad("sink").unwrap();
                assert_eq!(src_pad.link(&sink_pad), gst::PadLinkReturn::Ok);

                let audio_caps_mtx: Mutex<Option<AudioCaps>> = Mutex::new(None);
                let ctx_tx_arc_mtx_clone = ctx_tx_arc_mtx.clone();
                sink_pad.add_probe(gst::PAD_PROBE_TYPE_BUFFER, move |sink_pad, probe_info| {
                    if let Some(gst::PadProbeData::Buffer(ref buffer)) = probe_info.data {
                        // Retrieve audio caps the first time only
                        let mut audio_caps_opt = audio_caps_mtx.lock()
                            .expect("Failed to lock audio caps while receiving buffer");
                        let audio_caps = audio_caps_opt
                            .get_or_insert_with(|| AudioCaps::from_sink_pad(sink_pad));

                        let audio_buffer = AudioBuffer::from_gst_buffer(audio_caps, buffer);

                        ctx_tx_arc_mtx_clone.lock()
                            .expect("Failed to lock ctx_tx mutex, while transmitting audio buffer")
                            .send(ContextMessage::HaveAudioBuffer(audio_buffer))
                                .expect("Failed to transmit audio buffer");
                    };

                    gst::PadProbeReturn::Ok
                });
            } else if name.starts_with("video/") {
                let queue = gst::ElementFactory::make("queue", None).unwrap();
                let convert = gst::ElementFactory::make("videoconvert", None).unwrap();
                let scale = gst::ElementFactory::make("videoscale", None).unwrap();

                let (sink, widget_val) = if let Some(gtkglsink) = ElementFactory::make("gtkglsink", None) {
                    let glsinkbin = ElementFactory::make("glsinkbin", "video_sink").unwrap();
                    glsinkbin.set_property("sink", &gtkglsink.to_value()).unwrap();
                    let widget_val = gtkglsink.get_property("widget").unwrap();
                    (glsinkbin, widget_val)
                } else {
                    let sink = ElementFactory::make("gtksink", "video_sink").unwrap();
                    let widget_val = sink.get_property("widget").unwrap();
                    (sink, widget_val)
                };

                // Pass the video widget for integration in the UI
                ctx_tx_arc_mtx.lock()
                    .expect("Failed to lock ctx_tx mutex, while building the video queue")
                    .send(ContextMessage::HaveVideoWidget(widget_val))
                        .expect("Failed to transmit GstGtkWidget");
                // Wait for the widget to get added to the UI, otherwise the pipeline
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

                for e in elements { e.sync_state_with_parent().unwrap(); }

                let sink_pad = queue.get_static_pad("sink").unwrap();
                assert_eq!(src_pad.link(&sink_pad), gst::PadLinkReturn::Ok);
            }
        });
    }

    // Uses ctx_tx to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self, ctx_tx: Sender<ContextMessage>) {
        let info_arc_mtx = self.info.clone();
        let mut init_done = false;
        let bus = self.pipeline.get_bus().unwrap();
        bus.add_watch(move |_, msg| {
            // TODO: exit when pipeline status is null
            // or can we reuse the inspector for subsequent plays?
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    ctx_tx.send(ContextMessage::Eos)
                        .expect("Failed to notify UI");
                    glib::Continue(false)
                },
                gst::MessageView::Error(err) => {
                    eprintln!("Error from {}: {} ({:?})",
                        msg.get_src().get_path_string(),
                        err.get_error(), err.get_debug()
                    );
                    ctx_tx.send(ContextMessage::FailedToOpenMedia)
                        .expect("Failed to notify UI");
                    glib::Continue(false)
                },
                gst::MessageView::AsyncDone(_) => {
                    if !init_done {
                        ctx_tx.send(ContextMessage::InitDone)
                            .expect("Failed to notify UI");
                        init_done = true;
                    }
                    else {
                        ctx_tx.send(ContextMessage::AsyncDone)
                            .expect("Failed to notify UI");
                    }
                    glib::Continue(true)
                },
                gst::MessageView::Tag(msg_tag) => {
                    if !init_done {
                        let tags = msg_tag.get_tags();
                        let info = &mut info_arc_mtx.lock()
                            .expect("Failed to lock media info while reading tag data");
                        assign_str_tag!(info.title, tags, gst::tags::Title);
                        assign_str_tag!(info.artist, tags, gst::tags::Artist);
                        assign_str_tag!(info.artist, tags, gst::tags::AlbumArtist);
                        assign_str_tag!(info.container, tags, gst::tags::ContainerFormat);
                        assign_str_tag!(info.video_codec, tags, gst::tags::VideoCodec);
                        assign_str_tag!(info.audio_codec, tags, gst::tags::AudioCodec);

                        match tags.get::<gst::tags::PreviewImage>() {
                            // TODO: check if that happens, that would be handy for videos
                            Some(preview_tag) => println!("** Found a PreviewImage tag **"),
                            None => (),
                        };

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
                    glib::Continue(true)
                },
                gst::MessageView::Toc(msg_toc) => {
                    if init_done {
                        return glib::Continue(true);
                    }
                    let (toc, _) = msg_toc.get_toc();
                    if toc.get_scope() != TocScope::Global {
                        println!("Warning: Skipping toc with scope: {:?}", toc.get_scope());
                        return glib::Continue(true);
                    }

                    let info = &mut info_arc_mtx.lock()
                        .expect("Failed to lock media info while reading toc data");
                    if !info.chapters.is_empty() {
                        // chapters already retrieved
                        // TODO: check if there are medias with some sort of
                        // incremental tocs (not likely for files)
                        // or maybe the updated flag (_ above) should be used
                        return glib::Continue(true);
                    }

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
                                }
                                else {
                                    println!("Warning: Skipping toc sub entry with entry type: {:?}",
                                        sub_entry.get_entry_type()
                                    );
                                }
                            }
                        }
                        else {
                            println!("Warning: Skipping toc entry with entry type: {:?}",
                                entry.get_entry_type()
                            );
                        }
                    }

                    glib::Continue(true)
                }
                _ => glib::Continue(true),
            }
        });
    }
}
