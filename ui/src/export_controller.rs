use futures::channel::mpsc as async_mpsc;
use futures::future::LocalBoxFuture;
use futures::prelude::*;

use gettextrs::gettext;
use gtk::prelude::*;
use log::{error, warn};

use std::{
    fs,
    path::Path,
    rc::Rc,
    sync::{Arc, RwLock},
};

use media::{MediaEvent, TocSetterPipeline};
use metadata::{Duration, Exporter, Format, MatroskaTocFormat, MediaInfo};

use super::{PlaybackPipeline, UIController, UIEventSender, UIFocusContext};

use super::output_base_controller::{
    MediaEventHandling, MediaProcessorError, MediaProcessorImpl, OutputBaseController,
    OutputControllerImpl, OutputMediaFileInfo, ProcessingType, MEDIA_EVENT_CHANNEL_CAPACITY,
};

pub type ExportController = OutputBaseController<ExportControllerImpl>;

impl ExportController {
    pub fn new(builder: &gtk::Builder, ui_event: UIEventSender) -> Self {
        OutputBaseController::<ExportControllerImpl>::new_base(
            ExportControllerImpl::new(builder),
            builder,
            ui_event,
        )
    }
}

pub struct ExportControllerImpl {
    src_info: Option<Arc<RwLock<MediaInfo>>>,

    export_list: gtk::ListBox,
    mkvmerge_txt_row: gtk::ListBoxRow,
    mkvmerge_txt_warning_lbl: gtk::Label,
    cue_row: gtk::ListBoxRow,
    mkv_row: gtk::ListBoxRow,

    export_btn: gtk::Button,
}

impl OutputControllerImpl for ExportControllerImpl {
    type MediaProcessorImplType = ExportProcessor;

    const FOCUS_CONTEXT: UIFocusContext = UIFocusContext::ExportPage;
    const BTN_NAME: &'static str = "export-btn";
    const LIST_NAME: &'static str = "export-list-box";
    const PROGRESS_BAR_NAME: &'static str = "export-progress";

    fn new_processor(&self) -> ExportProcessor {
        let format = if self.mkvmerge_txt_row.is_selected() {
            Format::MKVMergeText
        } else if self.cue_row.is_selected() {
            Format::CueSheet
        } else if self.mkv_row.is_selected() {
            Format::Matroska
        } else {
            unreachable!("ExportControllerImpl::get_selected_format unknown export type");
        };

        ExportProcessor {
            src_info: Arc::clone(self.src_info.as_ref().unwrap()),
            idx: 0,
            export_file_info: Some({
                let src_info = self.src_info.as_ref().unwrap().read().unwrap();
                OutputMediaFileInfo::new(format, &src_info)
            }),
            toc_setter_pipeline: None,
        }
    }
}

impl UIController for ExportControllerImpl {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        self.src_info = Some(Arc::clone(&pipeline.info));
    }

    fn cleanup(&mut self) {
        self.src_info = None;
    }
}

impl ExportControllerImpl {
    pub fn new(builder: &gtk::Builder) -> Self {
        let ctrl = ExportControllerImpl {
            src_info: None,

            export_list: builder.get_object(Self::LIST_NAME).unwrap(),
            mkvmerge_txt_row: builder.get_object("mkvmerge_text_export-row").unwrap(),
            mkvmerge_txt_warning_lbl: builder.get_object("mkvmerge_text_warning-lbl").unwrap(),
            cue_row: builder.get_object("cue_sheet_export-row").unwrap(),
            mkv_row: builder.get_object("matroska_export-row").unwrap(),

            export_btn: builder.get_object(Self::BTN_NAME).unwrap(),
        };

        match TocSetterPipeline::check_requirements() {
            Ok(_) => ctrl.export_list.select_row(Some(&ctrl.mkvmerge_txt_row)),
            Err(err) => {
                warn!("{}", err);
                ctrl.mkvmerge_txt_warning_lbl.set_label(&err);

                ctrl.export_list.set_sensitive(false);
                ctrl.export_btn.set_sensitive(false);
            }
        }

        ctrl
    }
}

pub struct ExportProcessor {
    src_info: Arc<RwLock<MediaInfo>>,
    idx: usize,
    export_file_info: Option<OutputMediaFileInfo>,
    toc_setter_pipeline: Option<TocSetterPipeline>,
}

impl Iterator for ExportProcessor {
    type Item = Rc<Path>;

    fn next(&mut self) -> Option<Rc<Path>> {
        if self.idx > 0 {
            // ExportController outputs only one part
            return None;
        }

        let export_file_info = self.export_file_info.as_ref().unwrap();
        self.idx += 1;

        Some(Rc::clone(&export_file_info.path))
    }
}

impl MediaProcessorImpl for ExportProcessor {
    fn process<'a>(
        &'a mut self,
        output_path: &'a Path,
    ) -> LocalBoxFuture<'a, Result<ProcessingType, MediaProcessorError>> {
        async move {
            let format = self.export_file_info.as_ref().unwrap().format;
            match format {
                Format::MKVMergeText | Format::CueSheet => {
                    self.export_file_info = None;

                    let src_info = self.src_info.read().unwrap();
                    if src_info.toc.is_none() {
                        let msg = gettext("The table of contents is empty");
                        error!("{}", msg);
                        Err(msg)?;
                    }

                    // export toc as a standalone file
                    fs::File::create(&output_path)
                        .map_err(|_| gettext("Failed to create the file for the table of contents"))
                        .and_then(|mut output_file| {
                            metadata::Factory::writer(format)
                                .write(&src_info, &mut output_file)
                                .map_err(|msg| {
                                    let _ = fs::remove_file(&output_path);
                                    msg
                                })
                        })?;

                    Ok(ProcessingType::Sync)
                }
                Format::Matroska => {
                    let (sender, receiver) = async_mpsc::channel(MEDIA_EVENT_CHANNEL_CAPACITY);

                    let toc_setter_pipeline = TocSetterPipeline::try_new(
                        &self.src_info.read().unwrap().path,
                        output_path,
                        Arc::clone(&self.export_file_info.as_ref().unwrap().stream_ids),
                        sender,
                    )
                    .map_err(|err| {
                        gettext("Failed to prepare for export. {}").replacen("{}", &err, 1)
                    })?;

                    self.toc_setter_pipeline = Some(toc_setter_pipeline);
                    Ok(ProcessingType::Async(receiver))
                }
                format => unimplemented!("ExportControllerImpl for format {:?}", format),
            }
        }
        .boxed_local()
    }

    fn cancel(&mut self) {
        if let Some(pipeline) = self.toc_setter_pipeline.as_mut() {
            pipeline.cancel();

            if let Some(file_info) = self.export_file_info.take() {
                if std::fs::remove_file(&file_info.path).is_err() {
                    if let Some(printable_path) = file_info.path.to_str() {
                        warn!("Failed to remove canceled export file {}", printable_path);
                    }
                }
            }
        }
    }

    fn handle_media_event(
        &mut self,
        event: MediaEvent,
    ) -> Result<MediaEventHandling, MediaProcessorError> {
        match event {
            MediaEvent::InitDone => {
                let toc_setter_pipeline = self
                    .toc_setter_pipeline
                    .as_mut()
                    .expect("ExportController::handle_media_event no toc_setter_pipeline");

                let exporter = MatroskaTocFormat::new();
                {
                    let muxer = toc_setter_pipeline.muxer().unwrap();
                    let src_info = self.src_info.read().unwrap();
                    exporter.export(&src_info, muxer);
                }

                toc_setter_pipeline
                    .export()
                    .map_err(|err| gettext("Failed to export media. {}").replacen("{}", &err, 1))?;

                Ok(MediaEventHandling::ExpectingMore)
            }
            MediaEvent::Eos => {
                self.export_file_info = None;
                Ok(MediaEventHandling::Done)
            }
            MediaEvent::FailedToExport(err) => Err(gettext("Failed to export media. {}")
                .replacen("{}", &err, 1)
                .into()),
            other => unimplemented!("ExportController: can't handle media event {:?}", other),
        }
    }

    fn report_progress(&mut self) -> f64 {
        let duration = self.src_info.read().unwrap().duration;
        if duration > Duration::default() {
            self.toc_setter_pipeline
                .as_ref()
                .map(TocSetterPipeline::current_ts)
                .map_or(0f64, |ts| ts.as_f64() / duration.as_f64())
        } else {
            0f64
        }
    }

    fn completion_msg() -> String {
        gettext("Table of contents exported succesfully")
    }
}
