extern crate gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::{PadExt, QueryView};

extern crate glib;
use glib::ObjectExt;

use std::sync::mpsc::Sender;

use std::path::PathBuf;

use super::ContextMessage;

pub struct ExportContext {
    pipeline: gst::Pipeline,
    pub muxer: Option<gst::Element>,
    position_query: gst::Query,

    pub input_path: PathBuf,
    pub output_path: PathBuf,
}

impl ExportContext {
    pub fn new(
        input_path: PathBuf,
        output_path: PathBuf,
        ctx_tx: Sender<ContextMessage>,
    ) -> Result<ExportContext, String> {
        println!("\n\n* Exporting {:?} to {:?}...", input_path, output_path);

        let mut this = ExportContext {
            pipeline: gst::Pipeline::new("pipeline"),
            muxer: None,
            position_query: gst::Query::new_position(gst::Format::Time),

            input_path: input_path,
            output_path: output_path,
        };

        this.build_pipeline();
        this.register_bus_inspector(ctx_tx);

        match this.pipeline.set_state(gst::State::Paused) {
            gst::StateChangeReturn::Failure => Err("Could not set media in Paused state".into()),
            _ => Ok(this),
        }
    }

    pub fn get_muxer(&self) -> Option<&gst::Element> {
        self.muxer.as_ref()
    }

    pub fn export(&self) -> Result<(), String> {
        match self.pipeline.set_state(gst::State::Playing) {
            gst::StateChangeReturn::Failure => Err("Could not set media in palying state".into()),
            _ => Ok(()),
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.muxer.as_ref()
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
    ) {
        // Input
        let filesrc = gst::ElementFactory::make("filesrc", None).unwrap();
        filesrc.set_property(
            "location",
            &gst::Value::from(self.input_path.to_str().unwrap())
        ).unwrap();

        let parsebin = gst::ElementFactory::make("parsebin", None).unwrap();

        {
            self.pipeline.add_many(&[&filesrc, &parsebin]).unwrap();
            filesrc.link(&parsebin).unwrap();
            parsebin.sync_state_with_parent().unwrap();
        }

        // Muxer and output sink
        let muxer = gst::ElementFactory::make("matroskamux", None)
            .expect(
                concat!(
                    "ExportContext::build_pipeline couldn't find matroskamux plugin. ",
                    "Please install the gstreamer good plugins package"
                )
            );
        muxer.set_property("writing-app", &gst::Value::from("media-toc")).unwrap();

        let filesink = gst::ElementFactory::make("filesink", None).unwrap();
        filesink.set_property(
            "location",
            &gst::Value::from(self.output_path.to_str().unwrap())
        ).unwrap();

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
            assert_eq!(pad.link(&queue_sink_pad), gst::PadLinkReturn::Ok);

            let queue_src_pad = queue.get_static_pad("src").unwrap();
            let muxer_sink_pad = muxer.get_compatible_pad(&queue_src_pad, None).unwrap();
            assert_eq!(queue_src_pad.link(&muxer_sink_pad), gst::PadLinkReturn::Ok);

            for element in &[&queue, &muxer] {
                element.sync_state_with_parent().unwrap();
            }
        });
    }

    // Uses ctx_tx to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self, ctx_tx: Sender<ContextMessage>) {
        let mut init_done = false;
        self.pipeline.get_bus().unwrap().add_watch(move |_, msg| {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    ctx_tx.send(ContextMessage::Eos).expect(
                        "Failed to notify UI",
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
                    ctx_tx.send(ContextMessage::FailedToOpenMedia).expect(
                        "Failed to notify UI",
                    );
                    return glib::Continue(false);
                }
                gst::MessageView::AsyncDone(_) => {
                    if !init_done {
                        init_done = true;
                        ctx_tx.send(ContextMessage::InitDone).expect(
                            "Failed to notify UI",
                        );
                    } else {
                        ctx_tx.send(ContextMessage::AsyncDone).expect(
                            "Failed to notify UI",
                        );
                    }
                }
                _ => (),
            }

            glib::Continue(true)
        });
    }
}
