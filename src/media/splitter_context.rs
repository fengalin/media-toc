extern crate gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::{ClockTime, QueryView};

extern crate glib;

use std::sync::mpsc::Sender;

use std::path::Path;

use std::sync::{Arc, Mutex};

use super::ContextMessage;
use metadata;

pub struct SplitterContext {
    pipeline: gst::Pipeline,
    position_ref: Option<gst::Element>,
    position_query: gst::Query,

    format: metadata::Format,
    tags: gst::TagList,
}

impl SplitterContext {
    pub fn new(
        input_path: &Path,
        output_path: &Path,
        format: &metadata::Format,
        start: u64,
        end: u64,
        tags: gst::TagList,
        ctx_tx: Sender<ContextMessage>,
    ) -> Result<SplitterContext, String> {
        println!("\n\n* Exporting {:?} to {:?}...", input_path, output_path);

        let mut this = SplitterContext {
            pipeline: gst::Pipeline::new("pipeline"),
            position_ref: None,
            position_query: gst::Query::new_position(gst::Format::Time),

            format: format.clone(),
            tags: tags,
        };

        this.build_pipeline(input_path, output_path, start, end);
        this.register_bus_inspector(ctx_tx);

        match this.pipeline.set_state(gst::State::Paused) {
            gst::StateChangeReturn::Failure => Err("Could not set media in Paused state".into()),
            _ => Ok(this),
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.position_ref.as_ref().unwrap().query(
            self.position_query
                .get_mut()
                .unwrap(),
        );
        match self.position_query.view() {
            QueryView::Position(ref position) => position.get_result().get_value() as u64,
            _ => unreachable!(),
        }
    }

    // TODO: handle errors
    fn build_pipeline(&mut self, input_path: &Path, output_path: &Path, start: u64, end: u64) {
        /* There are multiple showstoppers to implementing something ideal
         * to export splitted chapters with audio and video (and subtitles):
         * 1. matroska-mux drops seek events explicitely (a message states: "discard for now").
         * This means that it is currently not possible to build a pipeline that would allow
         * seeking in a matroska media (using demux to interprete timestamps) and mux back to
         * to matroska. One solution would be to export streams to files and mux back
         * crossing fingers to make sure everything remains in sync.
         * 2. nlesrc (from gstreamer-editing-services) allows extracting frames from a starting
         * positiong for a given duration. However, it is designed to work with single stream
         * medias and decodes to raw formats.
         * 3. filesink can't change the file location without setting the pipeline to Null
         * which also unlinks elements.
         * 4. wavenc doesn't send header after a second seek in segment mode.
         * 5. flacenc can't handle a seek.
         *
         * Issues 3 & 4 lead to building a new pipeline for each chapter.
         *
         * Until I design a GUI for the user to select which stream to export to which codec,
         * current solution is to keep only the audio track and to save it as a flac file
         * which matches the initial purpose of this application */

        // Input
        let filesrc = gst::ElementFactory::make("filesrc", None).unwrap();
        filesrc
            .set_property("location", &gst::Value::from(input_path.to_str().unwrap()))
            .unwrap();
        let decodebin = gst::ElementFactory::make("decodebin", None).unwrap();

        self.pipeline
            .add_many(&[&filesrc, &decodebin])
            .unwrap();

        filesrc.link(&decodebin).unwrap();
        decodebin.sync_state_with_parent().unwrap();

        self.position_ref = Some(decodebin.clone());

        // Audio encoder
        let audio_enc = match self.format {
            metadata::Format::Flac => gst::ElementFactory::make("flacenc", None).unwrap(),
            metadata::Format::Wave => gst::ElementFactory::make("wavenc", None).unwrap(),
            metadata::Format::Opus => gst::ElementFactory::make("opusenc", None).unwrap(),
            metadata::Format::Vorbis => gst::ElementFactory::make("vorbisenc", None).unwrap(),
            _ => panic!("SplitterContext::build_pipeline unsupported format: {:?}", self.format),
        };

        // Catch events and drop the upstream Tags & TOC
        let audio_enc_sink_pad = audio_enc.get_static_pad("sink").unwrap();
        audio_enc_sink_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, |_pad, probe_info| {
            if let Some(ref data) = probe_info.data {
                match data {
                    &gst::PadProbeData::Event(ref event) => match event.view() {
                        gst::EventView::Tag(ref _tag) => return gst::PadProbeReturn::Drop,
                        gst::EventView::Toc(ref _toc) => return gst::PadProbeReturn::Drop,
                        _ => (),
                    },
                    _ => (),
                }
            }
            gst::PadProbeReturn::Ok
        });

        // Some encoders (such as flacenc) starts encoding the preroll buffers when switching to
        // paused mode. When the seek is performed buffers from the new segments are appended
        // to the ones from the preroll. Moreover, flacenc doesn't handle discontinuities.
        // As a workaround, we will drop buffers before the seek. The first buffer with the Discont
        // flag show that it is possible to seek. The next buffer with the Discont flag corresponds
        // to the first buffer from the target segment
        let seek_done = Arc::new(Mutex::new(false));
        let pipeline = self.pipeline.clone();
        audio_enc_sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, probe_info| {
            if let Some(ref data) = probe_info.data {
                match data {
                    &gst::PadProbeData::Buffer(ref buffer) => {
                        if buffer.get_flags() & gst::BufferFlags::DISCONT ==
                                gst::BufferFlags::DISCONT
                        {
                            let mut seek_done_grp = seek_done.lock()
                                .expect("audio_enc_sink_pad(buffer):: couldn't lock seek_done");
                            if *seek_done_grp == false {
                                // First buffer before seek
                                // let's seek and drop buffers until seek start sending new segment
                                match pipeline.seek(
                                    1f64,
                                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                                    gst::SeekType::Set,
                                    ClockTime::from(start),
                                    gst::SeekType::Set,
                                    ClockTime::from(end),
                                ) {
                                    Ok(_) => (),
                                    Err(_) => {
                                        eprintln!("Error: Failed to seek");
                                    }
                                };
                                *seek_done_grp = true;
                            } else {
                                // First Discont buffer after seek => stop dropping buffers
                                return gst::PadProbeReturn::Remove;
                            }
                        }
                    },
                    _ => (),
                }
            }
            gst::PadProbeReturn::Drop
        });

        self.pipeline.add(&audio_enc).unwrap();

        // add a muxer when required
        let (audio_muxer, tag_setter) = match self.format {
            metadata::Format::Flac | metadata::Format::Wave =>
                (audio_enc.clone(), audio_enc.clone()),
            metadata::Format::Opus | metadata::Format::Vorbis => {
                let ogg_muxer = gst::ElementFactory::make("oggmux", None).unwrap();
                self.pipeline.add(&ogg_muxer).unwrap();
                audio_enc.link(&ogg_muxer).unwrap();
                (ogg_muxer, audio_enc.clone())
            }
            _ => panic!("SplitterContext::build_pipeline unsupported format: {:?}", self.format),
        };

        //self.tag_setter = Some(tag_setter);
        let tag_setter = tag_setter
            .clone()
            .dynamic_cast::<gst::TagSetter>()
            .expect("SplitterContext::build_pipeline tag_setter is not a TagSetter");

        tag_setter.merge_tags(&self.tags, gst::TagMergeMode::ReplaceAll);

        // Output sink
        let outsink = gst::ElementFactory::make("filesink", "filesink").unwrap();
        outsink
            .set_property("location", &gst::Value::from(output_path.to_str().unwrap()))
            .unwrap();

        self.pipeline.add(&outsink).unwrap();
        audio_muxer.link(&outsink).unwrap();
        outsink.sync_state_with_parent().unwrap();

        let pipeline_cb = self.pipeline.clone();
        decodebin.connect_pad_added(move |_element, pad| {
            let caps = pad.get_current_caps().unwrap();
            let structure = caps.get_structure(0).unwrap();
            let name = structure.get_name();

            let queue = gst::ElementFactory::make("queue", None).unwrap();
            pipeline_cb.add(&queue).unwrap();
            let queue_sink_pad = queue.get_static_pad("sink").unwrap();
            assert_eq!(pad.link(&queue_sink_pad), gst::PadLinkReturn::Ok);
            queue.sync_state_with_parent().unwrap();
            let queue_src_pad = queue.get_static_pad("src").unwrap();

            if name.starts_with("audio/") && pipeline_cb.get_by_name("audioconvert").is_none() {
                let audio_conv = gst::ElementFactory::make("audioconvert", "audioconvert").unwrap();
                pipeline_cb.add(&audio_conv).unwrap();
                gst::Element::link_many(&[&queue, &audio_conv, &audio_enc]).unwrap();
                audio_conv.sync_state_with_parent().unwrap();
                audio_enc.sync_state_with_parent().unwrap();
            } else {
                let fakesink = gst::ElementFactory::make("fakesink", None).unwrap();
                pipeline_cb.add(&fakesink).unwrap();
                let fakesink_sink_pad = fakesink.get_static_pad("sink").unwrap();
                assert_eq!(
                    queue_src_pad.link(&fakesink_sink_pad),
                    gst::PadLinkReturn::Ok
                );
                fakesink.sync_state_with_parent().unwrap();
            }
        });
    }

    // Uses ctx_tx to notify the UI controllers
    fn register_bus_inspector(&self, ctx_tx: Sender<ContextMessage>) {
        let pipeline = self.pipeline.clone();
        self.pipeline.get_bus().unwrap().add_watch(move |_, msg| {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    if pipeline.set_state(gst::State::Null) == gst::StateChangeReturn::Failure {
                        ctx_tx.send(ContextMessage::FailedToExport).expect(
                            "Error: Failed to notify UI",
                        );
                    }
                    ctx_tx.send(ContextMessage::Eos).expect(
                        "Eos: Failed to notify UI",
                    );
                    return glib::Continue(false);
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
                    ctx_tx.send(ContextMessage::FailedToExport).expect(
                        "Error: Failed to notify UI",
                    );
                    return glib::Continue(false);
                }
                gst::MessageView::AsyncDone(_) => {
                    // Start splitting
                    if pipeline.set_state(gst::State::Playing) == gst::StateChangeReturn::Failure {
                        ctx_tx.send(ContextMessage::FailedToExport).expect(
                            "Error: Failed to notify UI",
                        );
                    }
                }
                _ => (),
            }

            glib::Continue(true)
        });
    }
}
