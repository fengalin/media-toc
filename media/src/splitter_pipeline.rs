use futures::channel::mpsc as async_mpsc;

use gettextrs::gettext;

use gst::{glib, prelude::*, ClockTime};

use log::{debug, error, info, warn};

use std::{
    convert::TryFrom,
    path::Path,
    sync::{Arc, Mutex},
};

use metadata::Format;
use renderers::Timestamp;

use super::MediaEvent;

pub struct SplitterPipeline {
    pipeline: gst::Pipeline,
    format: Format,
    chapter: gst::TocEntry,
}

impl SplitterPipeline {
    pub fn check_requirements(format: Format) -> Result<(), String> {
        match format {
            Format::Flac => {
                gst::ElementFactory::make("flacenc", None)
                    .map(drop)
                    .map_err(|_| {
                        gettext("Missing `{element}`\ncheck your gst-plugins-good install")
                            .replacen("{element}", "flacenc", 1)
                    })
            }
            Format::Wave => {
                gst::ElementFactory::make("wavenc", None)
                    .map(drop)
                    .map_err(|_| {
                        gettext("Missing `{element}`\ncheck your gst-plugins-good install")
                            .replacen("{element}", "wavenc", 1)
                    })
            }
            Format::Opus => {
                gst::ElementFactory::make("opusenc", None)
                    .map_err(|_| {
                        gettext("Missing `{element}`\ncheck your gst-plugins-good install")
                            .replacen("{element}", "opusenc", 1)
                    })
                    .and_then(|_| {
                        gst::ElementFactory::make("oggmux", None)
                            .map(drop)
                            .map_err(|_| {
                                gettext("Missing `{element}`\ncheck your gst-plugins-good install")
                                    .replacen("{element}", "oggmux", 1)
                            })
                    })
            }
            Format::Vorbis => {
                gst::ElementFactory::make("vorbisenc", None)
                    .map_err(|_| {
                        gettext("Missing `{element}`\ncheck your gst-plugins-good install")
                            .replacen("{element}", "vorbisenc", 1)
                    })
                    .and_then(|_| {
                        gst::ElementFactory::make("oggmux", None)
                            .map(drop)
                            .map_err(|_| {
                                gettext("Missing `{element}`\ncheck your gst-plugins-good install")
                                    .replacen("{element}", "oggmux", 1)
                            })
                    })
            }
            Format::MP3 => {
                gst::ElementFactory::make("lamemp3enc", None)
                    .map_err(|_| {
                        gettext("Missing `{element}`\ncheck your gst-plugins-good install")
                            .replacen("{element}", "lamemp3enc", 1)
                    })
                    .and_then(|_| {
                        gst::ElementFactory::make("id3v2mux", None)
                            .map(drop)
                            .map_err(|_| {
                                gettext("Missing `{element}`\ncheck your gst-plugins-good install")
                                    .replacen("{element}", "id3v2mux", 1)
                            })
                    })
            }
            _ => panic!(
                "SplitterPipeline::check_requirements unsupported format: {:?}",
                format
            ),
        }
    }

    pub fn try_new(
        input_path: &Path,
        output_path: &Path,
        stream_id: Option<String>,
        format: Format,
        chapter: gst::TocEntry,
        sender: async_mpsc::Sender<MediaEvent>,
    ) -> Result<SplitterPipeline, String> {
        info!(
            "{}",
            gettext("Splitting {}...").replacen("{}", output_path.to_str().unwrap(), 1)
        );
        debug!("stream id {:?}", stream_id);

        let mut this = SplitterPipeline {
            pipeline: gst::Pipeline::new(Some("splitter_pipeline")),
            format,
            chapter,
        };

        this.build_pipeline(input_path, output_path, stream_id);
        this.register_bus_inspector(sender);

        this.pipeline
            .set_state(gst::State::Paused)
            .map(|_| this)
            .map_err(|_| gettext("do you have permission to write the file?"))
    }

    pub fn current_ts(&self) -> Option<Timestamp> {
        // `current_ts` might be called when the pipeline is not ready yet
        // because we build a new pipeline for each split file.
        // When this happen, the ts position is negative so don't force it to 0
        // because we are most likely not at the begining of the source file.
        let mut position_query = gst::query::Position::new(gst::Format::Time);
        self.pipeline.query(&mut position_query);
        let position = position_query.result().value();
        if position >= 0 {
            Some(position.into())
        } else {
            None
        }
    }

    /// Builds the pipeline for split.
    ///
    /// If the `stream_id` is `None`, all the audio streams are used.
    fn build_pipeline(&mut self, input_path: &Path, output_path: &Path, stream_id: Option<String>) {
        /* There are multiple showstoppers to implementing something ideal
         * to export splitted chapters with audio and video (and subtitles):
         * 1. matroska-mux drops seek events explicitly (a message states: "discard for now").
         * This means that it is currently not possible to build a pipeline that would allow
         * seeking in a matroska media (using demux to interprete timestamps) and mux back to
         * to matroska. One solution would be to export streams to files and mux back
         * crossing fingers everything remains in sync.
         * 2. nlesrc (from gstreamer-editing-services) allows extracting frames from a starting
         * position for a given duration. However, it is designed to work with single stream
         * medias and decodes to raw formats.
         * 3. filesink can't change the file location without setting the pipeline to Null
         * which also unlinks elements.
         * 4. wavenc doesn't send header after a second seek in segment mode.
         * 5. flacenc can't handle a seek.
         *
         * Issues 3 & 4 lead to building a new pipeline for each chapter.
         *
         * Until I design a GUI for the user to select which stream to export to which codec,
         * current solution is to keep only the audio track which matches the initial purpose
         * of this application */

        // Input
        let filesrc = gst::ElementFactory::make("filesrc", None).unwrap();
        filesrc
            .set_property("location", &input_path.to_str().unwrap())
            .unwrap();
        let decodebin = gst::ElementFactory::make("decodebin", None).unwrap();

        self.pipeline.add_many(&[&filesrc, &decodebin]).unwrap();

        filesrc.link(&decodebin).unwrap();

        // Audio encoder
        let audio_enc = match self.format {
            Format::Flac => gst::ElementFactory::make("flacenc", None).unwrap(),
            Format::Wave => gst::ElementFactory::make("wavenc", None).unwrap(),
            Format::Opus => gst::ElementFactory::make("opusenc", None).unwrap(),
            Format::Vorbis => gst::ElementFactory::make("vorbisenc", None).unwrap(),
            Format::MP3 => gst::ElementFactory::make("lamemp3enc", None).unwrap(),
            _ => panic!(
                "SplitterPipeline::build_pipeline unsupported format: {:?}",
                self.format
            ),
        };

        use gst::PadProbeData::*;

        // Catch events and drop the upstream Tags & TOC
        let audio_enc_sink_pad = audio_enc.static_pad("sink").unwrap();
        audio_enc_sink_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, |_pad, probe_info| {
            if let Some(Event(ref event)) = probe_info.data {
                match event.view() {
                    gst::EventView::Tag(ref _tag) => return gst::PadProbeReturn::Drop,
                    gst::EventView::Toc(ref _toc) => return gst::PadProbeReturn::Drop,
                    _ => (),
                }
            }
            gst::PadProbeReturn::Ok
        });

        // Some encoders (such as flacenc) starts encoding the preroll buffers when switching to
        // paused mode. When the seek is performed buffers from the new segments are appended
        // to the ones from the preroll. Moreover, flacenc doesn't handle discontinuities.
        // As a workaround, we will drop buffers before the seek. The first buffer with the Discont
        // flag shows that it is possible to seek. The next buffer with the Discont flag corresponds
        // to the first buffer from the target segment

        let (start, end) = self.chapter.start_stop_times().unwrap();

        // Note: can't use AtomicBool here as pad probes are multithreaded so the function is Fn
        // not FnMut. See: https://github.com/sdroege/gstreamer-rs/pull/71
        #[allow(clippy::mutex_atomic)]
        let seek_done = Arc::new(Mutex::new(false));
        let pipeline = self.pipeline.clone();
        audio_enc_sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, probe_info| {
            if let Some(Buffer(ref buffer)) = probe_info.data {
                if buffer.flags() & gst::BufferFlags::DISCONT == gst::BufferFlags::DISCONT {
                    let mut seek_done_grp = seek_done.lock().unwrap();
                    if !*seek_done_grp {
                        // First buffer before seek
                        // let's seek and drop buffers until seek start sending new segment
                        let _res = pipeline
                            .seek(
                                1f64,
                                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                                gst::SeekType::Set,
                                ClockTime::try_from(start as u64).unwrap(),
                                gst::SeekType::Set,
                                ClockTime::try_from(end as u64).unwrap(),
                            )
                            .map_err(|_| {
                                // FIXME: feedback to the user using the UI channel
                                error!("{}", gettext("Failed to intialize the split"));
                            });
                        *seek_done_grp = true;
                    } else {
                        // First Discont buffer after seek => stop dropping buffers
                        return gst::PadProbeReturn::Remove;
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
            _ => unimplemented!(
                "SplitterPipeline::build_pipeline for format: {:?}",
                self.format
            ),
        };

        if let Some(tags) = self.chapter.tags() {
            let tag_setter = tag_setter.dynamic_cast::<gst::TagSetter>().unwrap();
            tag_setter.merge_tags(&tags, gst::TagMergeMode::ReplaceAll);
        }

        // Output sink
        let outsink = gst::ElementFactory::make("filesink", Some("filesink")).unwrap();
        outsink
            .set_property("location", &output_path.to_str().unwrap())
            .unwrap();

        self.pipeline.add(&outsink).unwrap();
        audio_muxer.link(&outsink).unwrap();

        let pipeline_cb = self.pipeline.clone();
        decodebin.connect_pad_added(move |_element, pad| {
            let caps = pad.current_caps().unwrap();
            let structure = caps.structure(0).unwrap();
            let name = structure.name();

            let is_selected_stream_id = stream_id.as_ref().map_or(true, |stream_id| {
                stream_id.as_str()
                    == pad
                        .stream_id()
                        .expect("SplitterPipeline::build_pipeline no stream_id for audio src pad")
            });

            if name.starts_with("audio/") && is_selected_stream_id {
                let audio_conv =
                    gst::ElementFactory::make("audioconvert", Some("audioconvert")).unwrap();
                pipeline_cb.add(&audio_conv).unwrap();
                pad.link(&audio_conv.static_pad("sink").unwrap()).unwrap();
                let audio_resample =
                    gst::ElementFactory::make("audioresample", Some("audioresample")).unwrap();
                pipeline_cb.add(&audio_resample).unwrap();
                gst::Element::link_many(&[&audio_conv, &audio_resample, &audio_enc]).unwrap();
                audio_conv.sync_state_with_parent().unwrap();
                audio_resample.sync_state_with_parent().unwrap();
                audio_enc.sync_state_with_parent().unwrap();
            } else {
                let fakesink = gst::ElementFactory::make("fakesink", None).unwrap();
                pipeline_cb.add(&fakesink).unwrap();
                pad.link(&fakesink.static_pad("sink").unwrap()).unwrap();
                fakesink.sync_state_with_parent().unwrap();
            }
        });
    }

    pub fn cancel(&self) {
        if self.pipeline.set_state(gst::State::Null).is_err() {
            warn!("could not stop the media");
        }
    }

    // Uses sender to notify the UI controllers
    fn register_bus_inspector(&self, mut sender: async_mpsc::Sender<MediaEvent>) {
        let pipeline = self.pipeline.clone();
        self.pipeline
            .bus()
            .unwrap()
            .add_watch(move |_, msg| {
                match msg.view() {
                    gst::MessageView::Eos(..) => {
                        if pipeline.set_state(gst::State::Null).is_err() {
                            sender
                                .try_send(MediaEvent::FailedToExport(gettext(
                                    "Failed to terminate properly. Check the resulting file.",
                                )))
                                .unwrap();
                        }
                        sender.try_send(MediaEvent::Eos).unwrap();
                        return glib::Continue(false);
                    }
                    gst::MessageView::Error(err) => {
                        let _ =
                            sender.try_send(MediaEvent::FailedToExport(err.error().to_string()));
                        return glib::Continue(false);
                    }
                    gst::MessageView::AsyncDone(_) => {
                        // Start splitting
                        if pipeline.set_state(gst::State::Playing).is_err() {
                            sender
                                .try_send(MediaEvent::FailedToExport(gettext(
                                    "Failed to start splitting.",
                                )))
                                .unwrap();
                        }
                    }
                    _ => (),
                }

                glib::Continue(true)
            })
            .unwrap();
    }
}
