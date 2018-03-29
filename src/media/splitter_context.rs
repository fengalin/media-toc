use gettextrs::gettext;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer::ClockTime;

use glib;

use std::error::Error;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;

use metadata::Format;

use super::ContextMessage;

pub struct SplitterContext {
    pipeline: gst::Pipeline,
    position_query: gst::query::Position<gst::Query>,

    format: Format,
    chapter: gst::TocEntry,
}

impl SplitterContext {
    pub fn check_requirements(format: Format) -> Result<(), String> {
        match format {
            Format::Flac => gst::ElementFactory::make("flacenc", None).map_or(
                Err(gettext(
                    "Missing `flacenc`\ncheck your gst-plugins-good install",
                )),
                |_| Ok(()),
            ),
            Format::Wave => gst::ElementFactory::make("wavenc", None).map_or(
                Err(gettext(
                    "Missing `wavenc`\ncheck your gst-plugins-good install",
                )),
                |_| Ok(()),
            ),
            Format::Opus => gst::ElementFactory::make("opusenc", None)
                .map_or(
                    Err(gettext(
                        "Missing `opusenc`\ncheck your gst-plugins-good install",
                    )),
                    |_| Ok(()),
                )
                .and_then(|_| {
                    gst::ElementFactory::make("oggmux", None).map_or(
                        Err(gettext(
                            "Missing `oggmux`\ncheck your gst-plugins-good install",
                        )),
                        |_| Ok(()),
                    )
                }),
            Format::Vorbis => gst::ElementFactory::make("vorbisenc", None)
                .map_or(
                    Err(gettext(
                        "Missing `opusenc`\ncheck your gst-plugins-good install",
                    )),
                    |_| Ok(()),
                )
                .and_then(|_| {
                    gst::ElementFactory::make("oggmux", None).map_or(
                        Err(gettext(
                            "Missing `oggmux`\ncheck your gst-plugins-good install",
                        )),
                        |_| Ok(()),
                    )
                }),
            Format::MP3 => gst::ElementFactory::make("lamemp3enc", None)
                .map_or(
                    Err(gettext(
                        "Missing `lamemp3enc`\ncheck your gst-plugins-good install",
                    )),
                    |_| Ok(()),
                )
                .and_then(|_| {
                    gst::ElementFactory::make("id3v2mux", None).map_or(
                        Err(gettext(
                            "Missing `id3v2mux`\ncheck your gst-plugins-good install",
                        )),
                        |_| Ok(()),
                    )
                }),
            _ => panic!(
                "SplitterContext::check_requirements unsupported format: {:?}",
                format
            ),
        }
    }

    pub fn new(
        input_path: &Path,
        output_path: &Path,
        format: Format,
        chapter: gst::TocEntry,
        ctx_tx: Sender<ContextMessage>,
    ) -> Result<SplitterContext, String> {
        info!(
            "{}",
            gettext("Splitting {}...").replacen("{}", output_path.to_str().unwrap(), 1)
        );

        let mut this = SplitterContext {
            pipeline: gst::Pipeline::new("pipeline"),
            position_query: gst::Query::new_position(gst::Format::Time),
            format,
            chapter,
        };

        this.build_pipeline(input_path, output_path);
        this.register_bus_inspector(ctx_tx);

        match this.pipeline.set_state(gst::State::Paused) {
            gst::StateChangeReturn::Failure => Err("Could not set media in Paused state".into()),
            _ => Ok(this),
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.pipeline.query(&mut self.position_query);
        self.position_query.get_result().get_value() as u64
    }

    #[cfg_attr(feature = "cargo-clippy", allow(mutex_atomic))]
    fn build_pipeline(&mut self, input_path: &Path, output_path: &Path) {
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

        self.pipeline.add_many(&[&filesrc, &decodebin]).unwrap();

        filesrc.link(&decodebin).unwrap();
        decodebin.sync_state_with_parent().unwrap();

        // Audio encoder
        let audio_enc = match self.format {
            Format::Flac => gst::ElementFactory::make("flacenc", None).unwrap(),
            Format::Wave => gst::ElementFactory::make("wavenc", None).unwrap(),
            Format::Opus => gst::ElementFactory::make("opusenc", None).unwrap(),
            Format::Vorbis => gst::ElementFactory::make("vorbisenc", None).unwrap(),
            Format::MP3 => gst::ElementFactory::make("lamemp3enc", None).unwrap(),
            _ => panic!(
                "SplitterContext::build_pipeline unsupported format: {:?}",
                self.format
            ),
        };

        // Catch events and drop the upstream Tags & TOC
        let audio_enc_sink_pad = audio_enc.get_static_pad("sink").unwrap();
        audio_enc_sink_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, |_pad, probe_info| {
            if let Some(ref data) = probe_info.data {
                if let gst::PadProbeData::Event(ref event) = *data {
                    match event.view() {
                        gst::EventView::Tag(ref _tag) => return gst::PadProbeReturn::Drop,
                        gst::EventView::Toc(ref _toc) => return gst::PadProbeReturn::Drop,
                        _ => (),
                    }
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

        // Note: can't use AtomicBool here as pad probes are multithreaded so the function is Fn
        // not FnMut. See: https://github.com/sdroege/gstreamer-rs/pull/71
        let (start, end) = self.chapter.get_start_stop_times().unwrap();
        let seek_done = Arc::new(Mutex::new(false));
        let pipeline = self.pipeline.clone();
        audio_enc_sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, probe_info| {
            if let Some(ref data) = probe_info.data {
                if let gst::PadProbeData::Buffer(ref buffer) = *data {
                    if buffer.get_flags() & gst::BufferFlags::DISCONT == gst::BufferFlags::DISCONT {
                        let mut seek_done_grp = seek_done.lock().unwrap();
                        if !*seek_done_grp {
                            // First buffer before seek
                            // let's seek and drop buffers until seek start sending new segment
                            match pipeline.seek(
                                1f64,
                                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                                gst::SeekType::Set,
                                ClockTime::from(start as u64),
                                gst::SeekType::Set,
                                ClockTime::from(end as u64),
                            ) {
                                Ok(_) => (),
                                Err(_) => {
                                    // FIXME: feedback to the user using the UI channel
                                    error!("{}", gettext("Failed to intialize the split"));
                                }
                            };
                            *seek_done_grp = true;
                        } else {
                            // First Discont buffer after seek => stop dropping buffers
                            return gst::PadProbeReturn::Remove;
                        }
                    }
                }
            }
            gst::PadProbeReturn::Drop
        });

        self.pipeline.add(&audio_enc).unwrap();

        // add a muxer when required
        let (tag_setter, audio_muxer) = match self.format {
            Format::Flac | Format::Wave => (audio_enc.clone(), audio_enc.clone()),
            Format::Opus | Format::Vorbis => {
                let ogg_muxer = gst::ElementFactory::make("oggmux", None).unwrap();
                self.pipeline.add(&ogg_muxer).unwrap();
                audio_enc.link(&ogg_muxer).unwrap();
                (audio_enc.clone(), ogg_muxer)
            }
            Format::MP3 => {
                let id3v2_muxer = gst::ElementFactory::make("id3v2mux", None).unwrap();
                self.pipeline.add(&id3v2_muxer).unwrap();
                audio_enc.link(&id3v2_muxer).unwrap();
                (id3v2_muxer.clone(), id3v2_muxer)
            }
            _ => panic!(
                "SplitterContext::build_pipeline unsupported format: {:?}",
                self.format
            ),
        };

        if let Some(tags) = self.chapter.get_tags() {
            let tag_setter = tag_setter.clone().dynamic_cast::<gst::TagSetter>().unwrap();
            tag_setter.merge_tags(&tags, gst::TagMergeMode::ReplaceAll);
        }

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
                        ctx_tx
                            .send(ContextMessage::FailedToExport(gettext(
                                "Failed to terminate properly. Check the resulting file.",
                            )))
                            .unwrap();
                    }
                    ctx_tx.send(ContextMessage::Eos).unwrap();
                    return glib::Continue(false);
                }
                gst::MessageView::Error(err) => {
                    ctx_tx
                        .send(ContextMessage::FailedToExport(
                            err.get_error().description().to_owned(),
                        ))
                        .unwrap();
                    return glib::Continue(false);
                }
                gst::MessageView::AsyncDone(_) => {
                    // Start splitting
                    if pipeline.set_state(gst::State::Playing) == gst::StateChangeReturn::Failure {
                        ctx_tx
                            .send(ContextMessage::FailedToExport(gettext(
                                "Failed to start splitting.",
                            )))
                            .unwrap();
                    }
                }
                _ => (),
            }

            glib::Continue(true)
        });
    }
}
