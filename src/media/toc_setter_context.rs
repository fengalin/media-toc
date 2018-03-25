use gettextrs::gettext;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::PadExt;

use glib;
use glib::ObjectExt;

use std::sync::mpsc::Sender;

use std::path::Path;

use super::ContextMessage;

pub struct TocSetterContext {
    pipeline: gst::Pipeline,
    muxer: Option<gst::Element>,
    position_query: gst::query::Position<gst::Query>,
}

impl TocSetterContext {
    pub fn check_requirements() -> Result<(), String> {
        // Exporting to Mastroska containers is only
        // available from gst-plugins-good 1.13.1
        let (major, minor, micro, _nano) = gst::version();
        if major >=1 && minor >=13 && micro >= 1 {
            gst::ElementFactory::make("matroskamux", None).map_or(
                Err(gettext("Missing `matroskamux`\ncheck your gst-plugins-good install")),
                |_| Ok(())
            )
        } else {
            Err(gettext("Matroska export requires\ngst-plugins-good >= 1.14"))
        }
    }

    pub fn new(
        input_path: &Path,
        output_path: &Path,
        ctx_tx: Sender<ContextMessage>,
    ) -> Result<TocSetterContext, String> {
        println!("\n\n* Exporting {:?} to {:?}...", input_path, output_path);

        let mut this = TocSetterContext {
            pipeline: gst::Pipeline::new("pipeline"),
            muxer: None,
            position_query: gst::Query::new_position(gst::Format::Time),
        };

        this.build_pipeline(input_path, output_path);
        this.register_bus_inspector(ctx_tx);

        match this.pipeline.set_state(gst::State::Paused) {
            gst::StateChangeReturn::Failure => Err("Could not set media in Paused state".into()),
            _ => Ok(this),
        }
    }

    pub fn get_muxer(&self) -> Option<&gst::Element> {
        self.muxer.as_ref()
    }

    pub fn export(&mut self) -> Result<(), String> {
        match self.pipeline.set_state(gst::State::Playing) {
            gst::StateChangeReturn::Failure => Err("Could not set media in palying state".into()),
            _ => Ok(()),
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.muxer
            .as_ref()
            .unwrap()
            .query(&mut self.position_query);
        self.position_query.get_result().get_value() as u64
    }

    // TODO: handle errors
    fn build_pipeline(&mut self, input_path: &Path, output_path: &Path) {
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
        let muxer = gst::ElementFactory::make("matroskamux", None).expect(concat!(
            "TocSetterContext::build_pipeline couldn't find matroskamux plugin. ",
            "Please install the gstreamer good plugins package"
        ));
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
            assert_eq!(pad.link(&queue_sink_pad), gst::PadLinkReturn::Ok);

            let queue_src_pad = queue.get_static_pad("src").unwrap();
            let muxer_sink_pad = muxer.get_compatible_pad(&queue_src_pad, None).unwrap();
            assert_eq!(queue_src_pad.link(&muxer_sink_pad), gst::PadLinkReturn::Ok);

            // Listen to incoming events and drop Upstream TOCs
            muxer_sink_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, |_pad, probe_info| {
                if let Some(ref data) = probe_info.data {
                    if let gst::PadProbeData::Event(ref event) = *data {
                        if let gst::EventView::Toc(ref _toc) = event.view() {
                            return gst::PadProbeReturn::Drop;
                        }
                    }
                }
                gst::PadProbeReturn::Ok
            });

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
                    ctx_tx
                        .send(ContextMessage::Eos)
                        .expect("Eos: Failed to notify UI");
                    return glib::Continue(false);
                }
                gst::MessageView::Error(err) => {
                    eprintln!(
                        "Error from {}: {} ({:?})",
                        msg.get_src()
                            .map(|s| s.get_path_string(),)
                            .unwrap_or_else(|| String::from("None"),),
                        err.get_error(),
                        err.get_debug()
                    );
                    ctx_tx
                        .send(ContextMessage::FailedToExport)
                        .expect("Error: Failed to notify UI");
                    return glib::Continue(false);
                }
                gst::MessageView::AsyncDone(_) => {
                    if !init_done {
                        init_done = true;
                        ctx_tx
                            .send(ContextMessage::InitDone)
                            .expect("InitDone: Failed to notify UI");
                    } else {
                        ctx_tx
                            .send(ContextMessage::AsyncDone)
                            .expect("AsyncDone: Failed to notify UI");
                    }
                }
                _ => (),
            }

            glib::Continue(true)
        });
    }
}
