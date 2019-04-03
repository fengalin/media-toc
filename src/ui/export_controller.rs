use gettextrs::gettext;
use glib;
use gtk;
use gtk::prelude::*;
use log::warn;

use std::{
    fs::File,
    path::Path,
    rc::Rc,
    sync::{Arc, RwLock},
};

use crate::{
    application::CommandLineArguments,
    media::{MediaEvent, PlaybackPipeline, TocSetterPipeline},
    metadata,
    metadata::{Exporter, Format, MatroskaTocFormat, MediaInfo},
};

use super::{
    MediaProcessor, OutputBaseController, OutputControllerImpl, OutputMediaFileInfo,
    ProcessingState, ProcessingType, UIController, UIEventSender,
};

pub type ExportController = OutputBaseController<ExportControllerImpl>;

impl ExportController {
    pub fn new(builder: &gtk::Builder, ui_event_sender: UIEventSender) -> Self {
        OutputBaseController::<ExportControllerImpl>::new_base(
            ExportControllerImpl::new(builder),
            builder,
            ui_event_sender,
        )
    }
}

pub struct ExportControllerImpl {
    src_info: Option<Arc<RwLock<MediaInfo>>>,

    idx: u64,
    export_file_info: Option<OutputMediaFileInfo>,
    media_event_sender: Option<glib::Sender<MediaEvent>>,
    toc_setter_pipeline: Option<TocSetterPipeline>,

    export_list: gtk::ListBox,
    mkvmerge_txt_row: gtk::ListBoxRow,
    mkvmerge_txt_warning_lbl: gtk::Label,
    cue_row: gtk::ListBoxRow,
    mkv_row: gtk::ListBoxRow,

    export_btn: gtk::Button,
}

impl OutputControllerImpl for ExportControllerImpl {
    const BTN_NAME: &'static str = "export-btn";
    const LIST_NAME: &'static str = "export-list-box";
    const PROGRESS_BAR_NAME: &'static str = "export-progress";
}

impl UIController for ExportControllerImpl {
    fn setup(&mut self, _args: &CommandLineArguments) {
        match TocSetterPipeline::check_requirements() {
            Ok(_) => self.export_list.select_row(Some(&self.mkvmerge_txt_row)),
            Err(err) => {
                warn!("{}", err);
                self.mkvmerge_txt_warning_lbl.set_label(&err);

                self.export_list.set_sensitive(false);
                self.export_btn.set_sensitive(false);
            }
        }
    }

    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        self.src_info = Some(Arc::clone(&pipeline.info));
    }

    fn cleanup(&mut self) {
        self.src_info = None;
        self.reset();
    }
}

impl ExportControllerImpl {
    pub fn new(builder: &gtk::Builder) -> Self {
        ExportControllerImpl {
            src_info: None,

            idx: 0,
            export_file_info: None,
            media_event_sender: None,
            toc_setter_pipeline: None,

            export_list: builder.get_object(Self::LIST_NAME).unwrap(),
            mkvmerge_txt_row: builder.get_object("mkvmerge_text_export-row").unwrap(),
            mkvmerge_txt_warning_lbl: builder.get_object("mkvmerge_text_warning-lbl").unwrap(),
            cue_row: builder.get_object("cue_sheet_export-row").unwrap(),
            mkv_row: builder.get_object("matroska_export-row").unwrap(),

            export_btn: builder.get_object(Self::BTN_NAME).unwrap(),
        }
    }

    fn reset(&mut self) {
        self.idx = 0;
        self.export_file_info = None;
        self.media_event_sender = None;
        self.toc_setter_pipeline = None;
    }
}

impl MediaProcessor for ExportControllerImpl {
    fn init(&mut self) -> ProcessingType {
        self.reset();

        let (format, processing_type) = if self.mkvmerge_txt_row.is_selected() {
            (Format::MKVMergeText, ProcessingType::Sync)
        } else if self.cue_row.is_selected() {
            (Format::CueSheet, ProcessingType::Sync)
        } else if self.mkv_row.is_selected() {
            let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
            self.media_event_sender = Some(sender);

            (Format::Matroska, ProcessingType::Async(receiver))
        } else {
            unreachable!("ExportControllerImpl::get_selected_format unknown export type");
        };

        self.export_file_info = Some({
            let src_info = self.src_info.as_ref().unwrap().read().unwrap();
            OutputMediaFileInfo::new(format, &src_info)
        });

        processing_type
    }

    fn next(&mut self) -> Result<ProcessingState, String> {
        if self.idx > 0 {
            // ExportController outputs only one part
            return Ok(ProcessingState::AllComplete(gettext(
                "Table of contents exported succesfully",
            )));
        }

        let export_file_info = self.export_file_info.as_ref().expect(concat!(
            "ExportControllerImpl: export_file_info not defined in `start()`, ",
            "did you call `init()`?"
        ));
        self.idx += 1;

        Ok(ProcessingState::WouldOutputTo(Rc::clone(
            &export_file_info.path,
        )))
    }

    fn process(&mut self, path: &Path) -> Result<ProcessingState, String> {
        let format = self
            .export_file_info
            .as_ref()
            .expect(concat!(
                "ExportControllerImpl: export_file_info not defined in `process()`, ",
                "did you call `init()`?"
            ))
            .format;
        let res = match format {
            Format::MKVMergeText | Format::CueSheet => {
                // export toc as a standalone file
                File::create(&path)
                    .map_err(|_| gettext("Failed to create the file for the table of contents"))
                    .and_then(|mut output_file| {
                        let res = {
                            let src_info = self.src_info.as_ref().unwrap().read().unwrap();
                            metadata::Factory::get_writer(format).write(&src_info, &mut output_file)
                        };
                        res.map(|_| {
                            self.export_file_info = None;
                            ProcessingState::DoneWithCurrent
                        })
                    })
            }
            Format::Matroska => {
                let res = TocSetterPipeline::try_new(
                    &self.src_info.as_ref().unwrap().read().unwrap().path,
                    path,
                    Arc::clone(&self.export_file_info.as_ref().unwrap().stream_ids),
                    self.media_event_sender
                        .as_ref()
                        .expect(
                            "ExportControllerImpl: no media_event_sender in `start()` did you call `init()`?",
                        )
                        .clone(),
                );

                res.map_err(|err| {
                    gettext("Failed to prepare for export. {}").replacen("{}", &err, 1)
                })
                .map(|toc_setter_pipeline| {
                    self.toc_setter_pipeline = Some(toc_setter_pipeline);
                    ProcessingState::PendingAsyncMediaEvent
                })
            }
            format => unimplemented!("ExportControllerImpl for format {:?}", format),
        };

        if res.is_err() {
            self.reset();
        }

        res
    }

    fn cancel(&mut self) {
        self.toc_setter_pipeline
            .as_mut()
            .expect("ExportController::cancel no toc_setter_pipeline")
            .cancel();

        {
            let path = &self
                .export_file_info
                .as_ref()
                .expect(concat!(
                    "ExportControllerImpl: export_file_info not defined in `cancel()`, ",
                    "did you call `init()`?"
                ))
                .path;

            if std::fs::remove_file(path).is_err() {
                if let Some(printable_path) = path.to_str() {
                    warn!("Failed to remove canceled export file {}", printable_path);
                }
            }
        }

        self.reset();
    }

    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingState, String> {
        match event {
            MediaEvent::InitDone => {
                let toc_setter_pipeline = self
                    .toc_setter_pipeline
                    .as_mut()
                    .expect("ExportController::handle_media_event no toc_setter_pipeline");

                let exporter = MatroskaTocFormat::new();
                {
                    let muxer = toc_setter_pipeline.get_muxer().unwrap();
                    let src_info = self.src_info.as_ref().unwrap().read().unwrap();
                    exporter.export(&src_info, muxer);
                }

                toc_setter_pipeline.export().map_err(|err| {
                    self.reset();
                    gettext("Failed to export media. {}").replacen("{}", &err, 1)
                })?;

                Ok(ProcessingState::PendingAsyncMediaEvent)
            }
            MediaEvent::Eos => {
                self.export_file_info = None;
                Ok(ProcessingState::DoneWithCurrent)
            }
            MediaEvent::FailedToExport(err) => {
                self.reset();
                Err(gettext("Failed to export media. {}").replacen("{}", &err, 1))
            }
            other => unimplemented!("ExportController: can't handle media event {:?}", other),
        }
    }

    fn report_progress(&self) -> Option<f64> {
        let duration = self.src_info.as_ref().unwrap().read().unwrap().duration;
        if duration > 0 {
            self.toc_setter_pipeline
                .as_ref()
                .map(|toc_setter_pipeline| toc_setter_pipeline.get_position())?
                .map(|position| position as f64 / duration as f64)
        } else {
            Some(0f64)
        }
    }
}
