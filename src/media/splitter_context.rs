extern crate gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::{ClockTime, PadExt, QueryView};

extern crate glib;
use glib::ObjectExt;

use std::sync::mpsc::Sender;

use std::path::Path;

use super::ContextMessage;

pub struct SplitterContext {
    pipeline: gst::Pipeline,
    filesink: Option<gst::Element>,
    position_ref: Option<gst::Element>,
    position_query: gst::Query,
}

impl SplitterContext {
    pub fn new(
        input_path: &Path,
        output_path: &Path,
        ctx_tx: Sender<ContextMessage>,
    ) -> Result<SplitterContext, String> {
        println!("\n\n* Exporting {:?} to {:?}...", input_path, output_path);

        let mut this = SplitterContext {
            pipeline: gst::Pipeline::new("pipeline"),
            filesink: None,
            position_ref: None,
            position_query: gst::Query::new_position(gst::Format::Time),
        };

        this.build_pipeline(input_path, output_path);
        this.register_bus_inspector(ctx_tx);

        match this.pipeline.set_state(gst::State::Paused) {
            gst::StateChangeReturn::Failure => Err("Could not set media in Paused state".into()),
            _ => Ok(this),
        }
    }

    pub fn export(&self) -> Result<(), String> {
        match self.pipeline.set_state(gst::State::Playing) {
            gst::StateChangeReturn::Failure => Err("Could not set media in palying state".into()),
            _ => {
                Ok(())
            },
        }
    }

    pub fn reset_output_path(&self, path: &Path) -> Result<(), ()> {
        match self.pipeline.set_state(gst::State::Null) {
            gst::StateChangeReturn::Failure => return Err(()),
            _ => (),
        };

        let filesink = self.filesink.as_ref()
            .expect("ExportController::export_part can't get filesink");
        filesink.set_property(
            "location",
            &gst::Value::from(path.with_extension("ogg").to_str().unwrap())
        ).unwrap();

        match self.pipeline.set_state(gst::State::Paused) {
            gst::StateChangeReturn::Failure => return Err(()),
            _ => (),
        };
        Ok(())
    }

    pub fn export_chapter(&self, start: u64, end: u64) -> Result<(), ()> {
        println!("seeking to {}, {}", start, end);

        match self.pipeline.seek(
            1f64,
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE | gst::SeekFlags::SEGMENT,
            gst::SeekType::Set,
            ClockTime::from(start),
            gst::SeekType::Set,
            ClockTime::from(end),
        ) {
            Ok(_) => { println!("seek ok"); Ok(()) },
            Err(_) => Err(()),
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.position_ref.as_ref()
            .unwrap()
            .query(self.position_query.get_mut().unwrap());
        match self.position_query.view() {
            QueryView::Position(ref position) => position.get_result().to_value() as u64,
            _ => unreachable!(),
        }
    }

    // TODO: handle errors
    fn build_pipeline(
        &mut self,
        input_path: &Path,
        output_path: &Path,
    ) {
        // FIXME: multiple issues
        /* There are multiple showstoppers to implementing something ideal
         * to export splitted chapters with audio and video (and subtitles):
         * - matroska-mux drops seek events explicitely (a message states: "discard for now").
         * This means that it is currently not possible to build a pipeline that would allow
         * seeking in a matroska media (using demux to interprete timstamp) and mux back to
         * to matroska. One solution would be to export streams to files and mux back
         * crossing fingers to make sure everything remains in sync.
         * - nlesrc (from gstreamer-editing-services) allows extracting frames from a starting
         * positiong for a given duration. However, it is designed to work with single stream
         * medias and decodes to raw formats.
         *
         * Until I design a GUI for the user to select which stream to export to which codec,
         * current solution is to keep only the audio track and to save it as a wave file */

        // Input
        let filesrc = gst::ElementFactory::make("filesrc", None).unwrap();
        filesrc.set_property(
            "location",
            &gst::Value::from(input_path.to_str().unwrap())
        ).unwrap();
        let decodebin = gst::ElementFactory::make("decodebin", None).unwrap();

        // Output sink
        let filesink = gst::ElementFactory::make("filesink", "filesink").unwrap();
        filesink.set_property(
            "location",
            &gst::Value::from(output_path.with_extension("wave").to_str().unwrap())
        ).unwrap();
        {
            self.pipeline.add_many(&[&filesrc, &decodebin, &filesink]).unwrap();
            filesrc.link(&decodebin).unwrap();
            decodebin.sync_state_with_parent().unwrap();
        }

        // TODO: add a tag setter element
        self.position_ref = Some(decodebin.clone());
        self.filesink = Some(filesink.clone());

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

            if name.starts_with("audio/") {
                println!("audio caps: {:?}", caps);
                let audio_conv = gst::ElementFactory::make("audioconvert", None).unwrap();
                let audio_enc = gst::ElementFactory::make("wavenc", None).unwrap();
                pipeline_cb.add_many(&[&audio_conv, &audio_enc]).unwrap();
                gst::Element::link_many(&[&queue, &audio_conv, &audio_enc, &filesink]).unwrap();
                audio_conv.sync_state_with_parent().unwrap();
                audio_enc.sync_state_with_parent().unwrap();
                filesink.sync_state_with_parent().unwrap();
            } else {
                let fakesink = gst::ElementFactory::make("fakesink", None).unwrap();
                pipeline_cb.add(&fakesink).unwrap();
                let fakesink_sink_pad = fakesink.get_static_pad("sink").unwrap();
                assert_eq!(queue_src_pad.link(&fakesink_sink_pad), gst::PadLinkReturn::Ok);
                fakesink.sync_state_with_parent().unwrap();
            }
        });
    }

    // Uses ctx_tx to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self, ctx_tx: Sender<ContextMessage>) {
        let mut init_done = false;
        let pipeline_cb = self.pipeline.clone();
        self.pipeline.get_bus().unwrap().add_watch(move |_, msg| {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    ctx_tx.send(ContextMessage::Eos).expect(
                        "Eos: Failed to notify UI",
                    );
                    return glib::Continue(false);
                }
                gst::MessageView::Error(err) => {
                    eprintln!(
                        "Error from {}: {} ({:?})",
                        msg.get_src().get_path_string(),
                        err.get_error(),
                        err.get_debug()
                    );
                    ctx_tx.send(ContextMessage::FailedToExport).expect(
                        "Error: Failed to notify UI",
                    );
                    return glib::Continue(false);
                }
                gst::MessageView::AsyncDone(_) => {
                    println!("gst::MessageView::AsyncDone");
                    if !init_done {
                        init_done = true;
                        ctx_tx.send(ContextMessage::InitDone).expect(
                            "InitDone: Failed to notify UI",
                        );
                    } else {
                        ctx_tx.send(ContextMessage::AsyncDone).expect(
                            "AsyncDone: Failed to notify UI",
                        );
                    }
                }
                gst::MessageView::StateChanged(state_changed) => {
                    println!("gst::MessageView::StateChanged {:?} => {:?} ({:?})",
                        state_changed.get_old(),
                        state_changed.get_current(),
                        state_changed.get_pending(),
                    );
                    if state_changed.get_current() == gst::State::Null {
                        init_done = false;
                    }
                }
                gst::MessageView::SegmentDone(_) => {
                    println!("gst::MessageView::SegmentDone");
                    match pipeline_cb.set_state(gst::State::Playing) {
                        gst::StateChangeReturn::Failure =>
                            ctx_tx.send(ContextMessage::FailedToExport).expect(
                                "Error: Failed to notify UI",
                            ),
                        _ => ()
                            /*ctx_tx.send(ContextMessage::SeekDone).expect(
                                "SegmentDone: Failed to notify UI",
                            )*/,
                    }
                }
                gst::MessageView::NewClock(_) => {
                    println!("gst::MessageView::NewClock");
                }
                _ => (()),
            }

            glib::Continue(true)
        });
    }
}
