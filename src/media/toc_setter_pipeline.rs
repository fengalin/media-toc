use gettextrs::gettext;

use gstreamer as gst;
use gstreamer::{prelude::*, PadExt};

use glib;
use glib::ObjectExt;

use log::info;

use std::{collections::HashSet, error::Error, path::Path, sync::mpsc::Sender};

use super::PipelineMessage;

pub struct TocSetterPipeline {
    pipeline: gst::Pipeline,
    muxer: Option<gst::Element>,
    position_query: gst::query::Position<gst::Query>,
}

impl TocSetterPipeline {
    pub fn check_requirements() -> Result<(), String> {
        // Exporting to Mastroska containers is only
        // available from gst-plugins-good 1.13.1
        let (major, minor, _micro, _nano) = gst::version();
        if major >= 1 && minor >= 14 {
            gst::ElementFactory::make("matroskamux", None).map_or(
                Err(gettext(
                    "Missing `matroskamux`\ncheck your gst-plugins-good install",
                )),
                |_| Ok(()),
            )
        } else {
            Err(gettext(
                "Matroska export requires\ngst-plugins-good >= 1.14",
            ))
        }
    }

    pub fn try_new(
        input_path: &Path,
        output_path: &Path,
        streams: HashSet<String>,
        pipeline_tx: Sender<PipelineMessage>,
    ) -> Result<TocSetterPipeline, String> {
        info!(
            "{}",
            gettext("Exporting to {}...").replacen("{}", output_path.to_str().unwrap(), 1)
        );

        let mut this = TocSetterPipeline {
            pipeline: gst::Pipeline::new("toc_setter_pipeline"),
            muxer: None,
            position_query: gst::Query::new_position(gst::Format::Time),
        };

        this.build_pipeline(input_path, output_path, streams);
        this.register_bus_inspector(pipeline_tx);

        this.pipeline
            .set_state(gst::State::Paused)
            .map(|_| this)
            .map_err(|_| gettext("Could not set media in Paused mode"))
    }

    pub fn get_muxer(&self) -> Option<&gst::Element> {
        self.muxer.as_ref()
    }

    pub fn export(&mut self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Playing)
            .map(|_| ())
            .map_err(|_| gettext("Could not set media in Playing mode"))
    }

    pub fn get_position(&mut self) -> u64 {
        self.pipeline.query(&mut self.position_query);
        self.position_query.get_result().get_value() as u64
    }

    fn build_pipeline(&mut self, input_path: &Path, output_path: &Path, streams: HashSet<String>) {
        // Input
        let filesrc = gst::ElementFactory::make("filesrc", None).unwrap();
        filesrc
            .set_property("location", &gst::Value::from(input_path.to_str().unwrap()))
            .unwrap();

        let parsebin = gst::ElementFactory::make("parsebin", None).unwrap();

        {
            self.pipeline.add_many(&[&filesrc, &parsebin]).unwrap();
            filesrc.link(&parsebin).unwrap();
            parsebin.sync_state_with_parent().unwrap();
        }

        // Muxer and output sink
        let muxer = gst::ElementFactory::make("matroskamux", None).unwrap();
        muxer
            .set_property("writing-app", &gst::Value::from("media-toc"))
            .unwrap();

        let filesink = gst::ElementFactory::make("filesink", "filesink").unwrap();
        filesink
            .set_property("location", &gst::Value::from(output_path.to_str().unwrap()))
            .unwrap();

        {
            self.pipeline.add_many(&[&muxer, &filesink]).unwrap();
            muxer.link(&filesink).unwrap();
            filesink.sync_state_with_parent().unwrap();
        }

        self.muxer = Some(muxer.clone());

        let pipeline_cb = self.pipeline.clone();
        parsebin.connect_pad_added(move |_element, pad| {
            let queue = gst::ElementFactory::make("queue", None).unwrap();
            pipeline_cb.add(&queue).unwrap();
            let queue_sink_pad = queue.get_static_pad("sink").unwrap();
            pad.link(&queue_sink_pad).unwrap();
            queue.sync_state_with_parent().unwrap();

            let queue_src_pad = queue.get_static_pad("src").unwrap();

            if streams.contains(
                pad.get_stream_id()
                    .expect("TocSetterPipeline::build_pipeline no stream_id for src pad")
                    .as_str(),
            ) {
                let muxer_sink_pad = muxer.get_compatible_pad(&queue_src_pad, None).unwrap();
                queue_src_pad.link(&muxer_sink_pad).unwrap();
                muxer.sync_state_with_parent().unwrap();

                // Listen to incoming events and drop Upstream TOCs
                muxer_sink_pad.add_probe(
                    gst::PadProbeType::EVENT_DOWNSTREAM,
                    |_pad, probe_info| {
                        if let Some(ref data) = probe_info.data {
                            if let gst::PadProbeData::Event(ref event) = *data {
                                if let gst::EventView::Toc(ref _toc) = event.view() {
                                    return gst::PadProbeReturn::Drop;
                                }
                            }
                        }
                        gst::PadProbeReturn::Ok
                    },
                );
            } else {
                let fakesink = gst::ElementFactory::make("fakesink", None).unwrap();
                pipeline_cb.add(&fakesink).unwrap();
                let fakesink_sink_pad = fakesink.get_static_pad("sink").unwrap();
                queue_src_pad.link(&fakesink_sink_pad).unwrap();
                fakesink.sync_state_with_parent().unwrap();
            }
        });
    }

    // Uses pipeline_tx to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self, pipeline_tx: Sender<PipelineMessage>) {
        let mut init_done = false;
        self.pipeline.get_bus().unwrap().add_watch(move |_, msg| {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    pipeline_tx.send(PipelineMessage::Eos).unwrap();
                    return glib::Continue(false);
                }
                gst::MessageView::Error(err) => {
                    pipeline_tx
                        .send(PipelineMessage::FailedToExport(
                            err.get_error().description().to_owned(),
                        ))
                        .unwrap();
                    return glib::Continue(false);
                }
                gst::MessageView::AsyncDone(_) => {
                    if !init_done {
                        init_done = true;
                        pipeline_tx.send(PipelineMessage::InitDone).unwrap();
                    } else {
                        pipeline_tx.send(PipelineMessage::AsyncDone).unwrap();
                    }
                }
                _ => (),
            }

            glib::Continue(true)
        });
    }
}
