use futures::channel::mpsc as async_mpsc;

use gettextrs::gettext;

use gst::{glib, prelude::*};

use log::{info, warn};

use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, RwLock},
};

use renderers::Timestamp;

use crate::MediaEvent;

pub struct TocSetter {
    pipeline: gst::Pipeline,
    muxer: Option<gst::Element>,
}

impl TocSetter {
    pub fn check_requirements() -> Result<(), String> {
        // Exporting to Mastroska containers is only
        // available from gst-plugins-good 1.13.1
        let (major, minor, _micro, _nano) = gst::version();
        if major >= 1 && minor >= 14 {
            gst::ElementFactory::make("matroskamux", None)
                .map(drop)
                .map_err(|_| {
                    gettext("Missing `{element}`\ncheck your gst-plugins-good install").replacen(
                        "{element}",
                        "matroskamux",
                        1,
                    )
                })
        } else {
            Err(gettext(
                "Matroska export requires\ngst-plugins-good >= 1.14",
            ))
        }
    }

    pub fn try_new(
        input_path: &Path,
        output_path: &Path,
        streams: Arc<RwLock<HashSet<String>>>,
        sender: async_mpsc::Sender<MediaEvent>,
    ) -> Result<TocSetter, String> {
        info!(
            "{}",
            gettext("Exporting to {}...").replacen("{}", output_path.to_str().unwrap(), 1)
        );

        let mut this = TocSetter {
            pipeline: gst::Pipeline::new(Some("toc_setter_pipeline")),
            muxer: None,
        };

        this.build_pipeline(input_path, output_path, streams);
        this.register_bus_inspector(sender);

        this.pipeline
            .set_state(gst::State::Paused)
            .map(|_| this)
            .map_err(|_| gettext("do you have permission to write the file?"))
    }

    pub fn muxer(&self) -> Option<&gst::Element> {
        self.muxer.as_ref()
    }

    pub fn export(&mut self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Playing)
            .map(|_| ())
            .map_err(|_| gettext("Could not set media in Playing mode"))
    }

    pub fn current_ts(&self) -> Timestamp {
        let mut position_query = gst::query::Position::new(gst::Format::Time);
        self.pipeline.query(&mut position_query);
        let position = position_query.result().value();
        if position >= 0 {
            position.into()
        } else {
            Timestamp::default()
        }
    }

    fn build_pipeline(
        &mut self,
        input_path: &Path,
        output_path: &Path,
        streams: Arc<RwLock<HashSet<String>>>,
    ) {
        // Input
        let filesrc = gst::ElementFactory::make("filesrc", None).unwrap();
        filesrc.set_property("location", &input_path.to_str().unwrap());

        let parsebin = gst::ElementFactory::make("parsebin", None).unwrap();

        self.pipeline.add_many(&[&filesrc, &parsebin]).unwrap();
        filesrc.link(&parsebin).unwrap();

        // Muxer and output sink
        let muxer = gst::ElementFactory::make("matroskamux", None).unwrap();
        muxer.set_property("writing-app", &"media-toc");

        let filesink = gst::ElementFactory::make("filesink", Some("filesink")).unwrap();
        filesink.set_property("location", &output_path.to_str().unwrap());

        self.pipeline.add_many(&[&muxer, &filesink]).unwrap();
        muxer.link(&filesink).unwrap();

        self.muxer = Some(muxer.clone());

        let pipeline_cb = self.pipeline.clone();
        parsebin.connect_pad_added(move |_element, pad| {
            let queue = gst::ElementFactory::make("queue2", None).unwrap();
            pipeline_cb.add(&queue).unwrap();

            pad.link(&queue.static_pad("sink").unwrap()).unwrap();
            queue.sync_state_with_parent().unwrap();

            let queue_src_pad = queue.static_pad("src").unwrap();

            if streams
                .read()
                .expect("TocSetter: `paserbin.pad_added` cand read streams to use")
                .contains(
                    pad.stream_id()
                        .expect("TocSetter::build_pipeline no stream_id for src pad")
                        .as_str(),
                )
            {
                let muxer_sink_pad = muxer.compatible_pad(&queue_src_pad, None).unwrap();
                queue_src_pad.link(&muxer_sink_pad).unwrap();
                muxer.sync_state_with_parent().unwrap();

                // Listen to incoming events and drop Upstream TOCs
                muxer_sink_pad.add_probe(
                    gst::PadProbeType::EVENT_DOWNSTREAM,
                    |_pad, probe_info| {
                        if let Some(gst::PadProbeData::Event(ref event)) = probe_info.data {
                            if let gst::EventView::Toc(ref _toc) = event.view() {
                                return gst::PadProbeReturn::Drop;
                            }
                        }
                        gst::PadProbeReturn::Ok
                    },
                );
            } else {
                let fakesink = gst::ElementFactory::make("fakesink", None).unwrap();
                pipeline_cb.add(&fakesink).unwrap();
                queue_src_pad
                    .link(&fakesink.static_pad("sink").unwrap())
                    .unwrap();
                fakesink.sync_state_with_parent().unwrap();
            }
        });
    }

    pub fn cancel(&self) {
        if self.pipeline.set_state(gst::State::Null).is_err() {
            warn!("could not stop the media");
        }
    }

    // Uses sender to notify the UI controllers about the inspection process
    fn register_bus_inspector(&self, mut sender: async_mpsc::Sender<MediaEvent>) {
        let mut init_done = false;
        self.pipeline
            .bus()
            .unwrap()
            .add_watch(move |_, msg| {
                match msg.view() {
                    gst::MessageView::Eos(..) => {
                        sender.try_send(MediaEvent::Eos).unwrap();
                        return glib::Continue(false);
                    }
                    gst::MessageView::Error(err) => {
                        let _ =
                            sender.try_send(MediaEvent::FailedToExport(err.error().to_string()));
                        return glib::Continue(false);
                    }
                    gst::MessageView::AsyncDone(_) => {
                        if !init_done {
                            init_done = true;
                            sender.try_send(MediaEvent::InitDone).unwrap();
                        }
                    }
                    _ => (),
                }

                glib::Continue(true)
            })
            .unwrap();
    }
}
