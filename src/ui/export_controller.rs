use gettextrs::gettext;
use glib;
use gtk;
use gtk::prelude::*;
use log::warn;

use std::{
    fs::File,
    sync::{Arc, RwLock},
};

use crate::{
    media::{MediaEvent, PlaybackPipeline, TocSetterPipeline},
    metadata,
    metadata::{Exporter, Format, MatroskaTocFormat, MediaInfo},
};

use super::{
    MediaProcessor, OutputBaseController, OutputControllerImpl, OutputMediaFileInfo,
    ProcessingStatus, ProcessingType, UIController,
};

pub type ExportController = OutputBaseController<ExportControllerImpl>;

impl ExportController {
    pub fn new(builder: &gtk::Builder) -> Self {
        OutputBaseController::<ExportControllerImpl>::new_base(
            ExportControllerImpl::new(builder),
            builder,
        )
    }
}

pub struct ExportControllerImpl {
    src_info: Option<Arc<RwLock<MediaInfo>>>,

    export_file_info: Option<OutputMediaFileInfo>,
    sender: Option<glib::Sender<MediaEvent>>,
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
    fn setup(&mut self) {
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
    }
}

impl ExportControllerImpl {
    pub fn new(builder: &gtk::Builder) -> Self {
        ExportControllerImpl {
            src_info: None,

            export_file_info: None,
            sender: None,
            toc_setter_pipeline: None,

            export_list: builder.get_object(Self::LIST_NAME).unwrap(),
            mkvmerge_txt_row: builder.get_object("mkvmerge_text_export-row").unwrap(),
            mkvmerge_txt_warning_lbl: builder.get_object("mkvmerge_text_warning-lbl").unwrap(),
            cue_row: builder.get_object("cue_sheet_export-row").unwrap(),
            mkv_row: builder.get_object("matroska_export-row").unwrap(),

            export_btn: builder.get_object(Self::BTN_NAME).unwrap(),
        }
    }
}

impl MediaProcessor for ExportControllerImpl {
    fn init(&mut self) -> ProcessingType {
        let (format, processing_type) = if self.mkvmerge_txt_row.is_selected() {
            (Format::MKVMergeText, ProcessingType::Sync)
        } else if self.cue_row.is_selected() {
            (Format::CueSheet, ProcessingType::Sync)
        } else if self.mkv_row.is_selected() {
            let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
            self.sender = Some(sender);

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

    fn start(&mut self) -> Result<ProcessingStatus, String> {
        let export_file_info = self.export_file_info.as_ref().expect(concat!(
            "ExportControllerImpl: export_file_info not defined in `start()`, ",
            "did you call `init()`?"
        ));
        match export_file_info.format {
            Format::MKVMergeText | Format::CueSheet => {
                // export toc as a standalone file
                let mut output_file = File::create(&export_file_info.path)
                    .map_err(|_| gettext("Failed to create the file for the table of contents"))?;
                let src_info = self.src_info.as_ref().unwrap().read().unwrap();
                metadata::Factory::get_writer(export_file_info.format)
                    .write(&src_info, &mut output_file)?;

                self.export_file_info = None;
                Ok(ProcessingStatus::Completed(gettext(
                    "Table of contents exported succesfully",
                )))
            }
            Format::Matroska => {
                let toc_setter_pipeline = TocSetterPipeline::try_new(
                    &self.src_info.as_ref().unwrap().read().unwrap().path,
                    &export_file_info.path,
                    Arc::clone(&export_file_info.stream_ids),
                    self.sender
                        .as_ref()
                        .expect(
                            "ExportControllerImpl: no sender in `start()` did you call `init()`?",
                        )
                        .clone(),
                )
                .map_err(|err| {
                    gettext("Failed to prepare for export. {}").replacen("{}", &err, 1)
                })?;
                self.toc_setter_pipeline = Some(toc_setter_pipeline);

                Ok(ProcessingStatus::InProgress)
            }
            format => unimplemented!("ExportControllerImpl for format {:?}", format),
        }
    }

    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingStatus, String> {
        match event {
            MediaEvent::InitDone => {
                let toc_setter_pipeline = self.toc_setter_pipeline.as_mut().unwrap();

                let exporter = MatroskaTocFormat::new();
                {
                    let muxer = toc_setter_pipeline.get_muxer().unwrap();
                    let src_info = self.src_info.as_ref().unwrap().read().unwrap();
                    exporter.export(&src_info, muxer);
                }

                toc_setter_pipeline
                    .export()
                    .map_err(|err| gettext("Failed to export media. {}").replacen("{}", &err, 1))?;

                Ok(ProcessingStatus::InProgress)
            }
            MediaEvent::Eos => {
                self.export_file_info = None;
                Ok(ProcessingStatus::Completed(gettext(
                    "Media exported succesfully",
                )))
            }
            MediaEvent::FailedToExport(err) => {
                self.export_file_info = None;
                Err(gettext("Failed to export media. {}").replacen("{}", &err, 1))
            }
            _ => Ok(ProcessingStatus::InProgress),
        }
    }

    fn report_progress(&mut self) -> Option<f64> {
        let duration = self.src_info.as_ref().unwrap().read().unwrap().duration;
        if duration > 0 {
            self.toc_setter_pipeline
                .as_mut()
                .map(|toc_setter_pipeline| toc_setter_pipeline.get_position())?
                .map(|position| position as f64 / duration as f64)
        } else {
            Some(0f64)
        }
    }
}
