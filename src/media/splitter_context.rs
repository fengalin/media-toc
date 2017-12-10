extern crate gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::{ClockTime, QueryView, TocSetterExt};

extern crate glib;

use std::sync::mpsc::Sender;

use std::path::Path;

use super::ContextMessage;

pub struct SplitterContext {
    pipeline: gst::Pipeline,
    tag_toc_element: Option<gst::Element>,
    position_ref: Option<gst::Element>,
    position_query: gst::Query,
}

impl SplitterContext {
    pub fn new(
        input_path: &Path,
        output_path: &Path,
        start: u64,
        end: u64,
        ctx_tx: Sender<ContextMessage>,
    ) -> Result<SplitterContext, String> {
        println!("\n\n* Exporting {:?}...", input_path);

        let mut this = SplitterContext {
            pipeline: gst::Pipeline::new("pipeline"),
            tag_toc_element: None,
            position_ref: None,
            position_query: gst::Query::new_position(gst::Format::Time),
        };

        this.build_pipeline(input_path, output_path);
        this.register_bus_inspector(start, end, ctx_tx);

        match this.pipeline.set_state(gst::State::Paused) {
            gst::StateChangeReturn::Failure => Err("Could not set media in Paused state".into()),
            _ => Ok(this),
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.position_ref.as_ref()
            .unwrap()
            .query(self.position_query.get_mut().unwrap());
        match self.position_query.view() {
            QueryView::Position(ref position) => position.get_result().get_value() as u64,
            _ => unreachable!(),
        }
    }

    // TODO: handle errors
    fn build_pipeline(&mut self,
        input_path: &Path,
        output_path: &Path,
    ) {
        // FIXME: multiple issues
        /* There are multiple showstoppers to implementing something ideal
         * to export splitted chapters with audio and video (and subtitles):
         * - matroska-mux drops seek events explicitely (a message states: "discard for now").
         * This means that it is currently not possible to build a pipeline that would allow
         * seeking in a matroska media (using demux to interprete timestamps) and mux back to
         * to matroska. One solution would be to export streams to files and mux back
         * crossing fingers to make sure everything remains in sync.
         * - nlesrc (from gstreamer-editing-services) allows extracting frames from a starting
         * positiong for a given duration. However, it is designed to work with single stream
         * medias and decodes to raw formats.
         * - filesink can't change the file location without setting the pipeline to Null
         * which also unlinks elements.
         * - wavenc doesn't send header after a second seek in segment mode.
         *
         * The two last issues lead to building a new pipeline for each chapter.
         *
         * Until I design a GUI for the user to select which stream to export to which codec,
         * current solution is to keep only the audio track and to save it as a wave file
         * which matches the initial purpose of this application */

        // Input
        let filesrc = gst::ElementFactory::make("filesrc", None).unwrap();
        filesrc.set_property(
            "location",
            &gst::Value::from(input_path.to_str().unwrap())
        ).unwrap();
        let decodebin = gst::ElementFactory::make("decodebin", None).unwrap();

        // Output sink
        let audio_enc = gst::ElementFactory::make("wavenc", "audioenc").unwrap();
        let outsink = gst::ElementFactory::make("filesink", "filesink").unwrap();
        outsink.set_property(
            "location",
            &gst::Value::from(output_path.with_extension("wave").to_str().unwrap())
        ).unwrap();

        {
            self.pipeline.add_many(&[&filesrc, &decodebin, &audio_enc, &outsink]).unwrap();
            filesrc.link(&decodebin).unwrap();
            decodebin.sync_state_with_parent().unwrap();
            audio_enc.link(&outsink).unwrap();
            outsink.sync_state_with_parent().unwrap();
        }

        self.position_ref = Some(decodebin.clone());
        self.tag_toc_element = Some(audio_enc.clone());

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
                assert_eq!(queue_src_pad.link(&fakesink_sink_pad), gst::PadLinkReturn::Ok);
                fakesink.sync_state_with_parent().unwrap();
            }
        });
    }

    // Uses ctx_tx to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self,
        start: u64,
        end: u64,
        ctx_tx: Sender<ContextMessage>,
    ) {
        let mut init_done = false;
        let pipeline = self.pipeline.clone();
        let tag_toc_element = self.tag_toc_element.as_ref()
            .expect("SplitContext::register_bus_inspector no tag toc element")
            .clone();
        self.pipeline.get_bus().unwrap().add_watch(move |_, msg| {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    match pipeline.set_state(gst::State::Null) {
                        gst::StateChangeReturn::Failure =>
                            ctx_tx.send(ContextMessage::FailedToExport).expect(
                                "Error: Failed to notify UI",
                            ),
                        _ => (),
                    }
                    init_done = false;
                    ctx_tx.send(ContextMessage::Eos).expect(
                        "Eos: Failed to notify UI",
                    );
                    return glib::Continue(false);
                }
                gst::MessageView::Error(err) => {
                    eprintln!(
                        "Error from {}: {} ({:?})",
                        msg.get_src()
                            .map(|s| s.get_path_string())
                            .unwrap_or_else(|| String::from("None")),
                        err.get_error(),
                        err.get_debug()
                    );
                    ctx_tx.send(ContextMessage::FailedToExport).expect(
                        "Error: Failed to notify UI",
                    );
                    return glib::Continue(false);
                }
                gst::MessageView::AsyncDone(_) => {
                    if !init_done {
                        init_done = true;

                        // TODO: set tags
                        let tag_setter =
                            tag_toc_element.clone().dynamic_cast::<gst::TagSetter>()
                                .expect("SplitterContext(AsyncDone) not a TagSetter");
                        tag_setter.reset_tags();

                        // Don't export initial media's toc
                        // FIXME: actually remove the toc (see wavenc source code)
                        let toc_setter =
                            tag_toc_element.clone().dynamic_cast::<gst::TocSetter>()
                                .expect("SplitterContext(AsyncDone) not a TocSetter");
                        toc_setter.reset();

                        match pipeline.seek(
                            1f64,
                            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                            gst::SeekType::Set,
                            ClockTime::from(start),
                            gst::SeekType::Set,
                            ClockTime::from(end),
                        ) {
                            Ok(_) => (),
                            Err(_) =>
                                ctx_tx.send(ContextMessage::FailedToExport).expect(
                                    "Error: Failed to notify UI",
                            ),
                        };
                    } else {
                        match pipeline.set_state(gst::State::Playing) {
                            gst::StateChangeReturn::Failure =>
                                ctx_tx.send(ContextMessage::FailedToExport).expect(
                                    "Error: Failed to notify UI",
                            ),
                            _ => ()
                        }
                    }
                }
                _ => (()),
            }

            glib::Continue(true)
        });
    }
}
