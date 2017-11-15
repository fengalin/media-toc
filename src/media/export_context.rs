extern crate gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::PadExt;

extern crate glib;
use glib::ObjectExt;

use std::path::PathBuf;

pub struct ExportContext {
    pipeline: gst::Pipeline,
    pub muxer: Option<gst::Element>,

    pub input_path: PathBuf,
    pub output_path: PathBuf,
}

impl ExportContext {
    pub fn new(
        input_path: PathBuf,
        output_path: PathBuf,
    ) -> Result<ExportContext, String> {
        println!("\n\n* Exporting {:?} to {:?}...", input_path, output_path);

        let mut ctx = ExportContext {
            pipeline: gst::Pipeline::new("pipeline"),
            muxer: None,

            input_path: input_path,
            output_path: output_path,
        };

        ctx.build_pipeline();

        /*match ctx.pipeline.set_state(gst::State::Paused) {
            gst::StateChangeReturn::Failure => Err("Could not set media in Paused state".into()),
            _ => Ok(ctx),
        }*/
        match ctx.pipeline.set_state(gst::State::Playing) {
            gst::StateChangeReturn::Failure => Err("Could not set media in palying state".into()),
            _ => Ok(ctx),
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
}
