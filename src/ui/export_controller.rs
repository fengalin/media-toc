use gettextrs::gettext;
use glib;
use gtk;
use gtk::prelude::*;
use log::{error, warn};

use std::{
    cell::RefCell,
    fs::File,
    ops::{Deref, DerefMut},
    rc::{Rc, Weak},
};

use crate::{
    media::{MediaEvent, PlaybackPipeline, TocSetterPipeline},
    metadata,
    metadata::{Exporter, Format, MatroskaTocFormat},
};

use super::{
    MainController, OutputBaseController, OutputProcessor, OutputUIController, ProcessorStatus, UIController,
};

const TIMER_PERIOD: u32 = 100; // 100 ms (10 Hz)

#[derive(Clone, PartialEq)]
enum ExportType {
    ExternalToc,
    SingleFileWithToc,
}

pub struct ExportController {
    base: OutputBaseController,

    export_list: gtk::ListBox,
    mkvmerge_txt_row: gtk::ListBoxRow,
    mkvmerge_txt_warning_lbl: gtk::Label,
    cue_row: gtk::ListBoxRow,
    mkv_row: gtk::ListBoxRow,
    export_progress_bar: gtk::ProgressBar,
    export_btn: gtk::Button,

    toc_setter_pipeline: Option<TocSetterPipeline>,
    this_opt: Option<Weak<RefCell<ExportController>>>,
}

impl UIController for ExportController {
    fn new_media(&mut self, _pipeline: &PlaybackPipeline) {
        self.export_btn.set_sensitive(true);
    }

    fn cleanup(&mut self) {
        self.export_btn.set_sensitive(false);
        self.export_progress_bar.set_fraction(0f64);
    }
}

impl ExportController {
    pub fn new_rc(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(ExportController {
            base: OutputBaseController::new(builder),

            export_list: builder.get_object("export-list-box").unwrap(),
            mkvmerge_txt_row: builder.get_object("mkvmerge_text_export-row").unwrap(),
            mkvmerge_txt_warning_lbl: builder.get_object("mkvmerge_text_warning-lbl").unwrap(),
            cue_row: builder.get_object("cue_sheet_export-row").unwrap(),
            mkv_row: builder.get_object("matroska_export-row").unwrap(),
            export_progress_bar: builder.get_object("export-progress").unwrap(),
            export_btn: builder.get_object("export-btn").unwrap(),

            toc_setter_pipeline: None,
            this_opt: None,
        }));

        {
            let mut this_mut = this.borrow_mut();
            this_mut.this_opt = Some(Rc::downgrade(&this));

            match TocSetterPipeline::check_requirements() {
                Ok(_) => this_mut.export_list.select_row(&this_mut.mkvmerge_txt_row),
                Err(err) => {
                    warn!("{}", err);
                    this_mut.mkvmerge_txt_warning_lbl.set_label(&err);

                    this_mut.export_list.set_sensitive(false);
                    this_mut.export_btn.set_sensitive(false);
                }
            }

            this_mut.cleanup();
        }

        this
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();
        this.have_main_ctrl(main_ctrl);

        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.export_btn.connect_clicked(move |_| {
            let this_clone = Rc::clone(&this_clone);
            main_ctrl_clone
                .borrow_mut()
                .request_pipeline(Box::new(move |pipeline| {
                    {
                        this_clone.borrow_mut().playback_pipeline = Some(pipeline);
                    }
                    // launch export asynchronoulsy so that main_ctrl is no longer borrowed
                    let this_clone = Rc::clone(&this_clone);
                    gtk::idle_add(move || {
                        let mut this_mut = this_clone.borrow_mut();
                        this_mut.switch_to_busy();
                        let is_in_progress = match this_mut.start() {
                            Ok(ProcessorStatus::Completed(msg)) => {
                                this_mut.show_info(msg);
                                false
                            }
                            Ok(ProcessorStatus::InProgress) => true,
                            Err(err) => {
                                error!("{}", err);
                                this_mut.show_error(err);
                                false
                            }
                        };

                        if !is_in_progress {
                            this_mut.restore_pipeline();
                            this_mut.switch_to_available();
                        }

                        glib::Continue(false)
                    });
                }));
        });
    }

    fn get_selection(&self) -> (metadata::Format, ExportType) {
        if self.mkvmerge_txt_row.is_selected() {
            (Format::MKVMergeText, ExportType::ExternalToc)
        } else if self.cue_row.is_selected() {
            (Format::CueSheet, ExportType::ExternalToc)
        } else if self.mkv_row.is_selected() {
            (Format::Matroska, ExportType::SingleFileWithToc)
        } else {
            unreachable!("ExportController::get_export_selection unknown export type");
        }
    }

    fn register_timer(&mut self, period: u32) {
        let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

        self.timer_src = Some(glib::timeout_add_local(period, move || {
            let this_rc = this_weak
                .upgrade()
                .expect("Lost `ExportController` in timer");
            this_rc.borrow_mut().update_progress();

            glib::Continue(true)
        }));
    }

    fn register_media_event_handler(&mut self, receiver: glib::Receiver<MediaEvent>) {
        let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

        receiver.attach(None, move |event| {
            let this_rc = this_weak
                .upgrade()
                .expect("Lost `ExportController` in `MediaEvent` handler");
            let mut this = this_rc.borrow_mut();
            let is_in_progress = match this.handle_media_event(event) {
                Ok(ProcessorStatus::Completed(msg)) => {
                    this.show_info(msg);
                    false
                }
                Ok(ProcessorStatus::InProgress) => true,
                Err(err) => {
                    this.show_error(err);
                    false
                }
            };

            if is_in_progress {
                glib::Continue(true)
            } else {
                this.restore_pipeline();
                this.switch_to_available();
                glib::Continue(false)
            }
        });
    }
}

impl OutputProcessor for ExportController {
    fn start(&mut self) -> Result<ProcessorStatus, String> {
        debug_assert!(self.playback_pipeline.is_some());
        let (format, export_type) = self.get_selection();

        let (stream_ids, content) = self
            .playback_pipeline
            .as_ref()
            .unwrap()
            .info
            .read()
            .unwrap()
            .get_stream_ids_to_export();

        self.prepare_process(format, content);

        match export_type {
            ExportType::ExternalToc => {
                // export toc as a standalone file
                let mut output_file = File::create(&self.target_path)
                    .map_err(|_| gettext("Failed to create the file for the table of contents"))?;
                let info = self
                    .playback_pipeline
                    .as_ref()
                    .unwrap()
                    .info
                    .read()
                    .unwrap();
                metadata::Factory::get_writer(format).write(&info, &mut output_file)?;
                Ok(ProcessorStatus::Completed(gettext("Table of contents exported succesfully")))
            }
            ExportType::SingleFileWithToc => {
                let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
                // FIXME: find a way to register outside for Async Processings
                self.register_media_event_handler(receiver);
                self.register_timer(TIMER_PERIOD);

                let toc_setter_pipeline = TocSetterPipeline::try_new(
                    &self.media_path,
                    &self.target_path,
                    stream_ids,
                    sender,
                ).map_err(|err| {
                    gettext("Failed to prepare for export. {}").replacen("{}", &err, 1)
                })?;
                self.toc_setter_pipeline = Some(toc_setter_pipeline);

                Ok(ProcessorStatus::InProgress)
            }
        }
    }

    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessorStatus, String> {
        match event {
            MediaEvent::InitDone => {
                let mut toc_setter_pipeline = self.toc_setter_pipeline.take().unwrap();

                let exporter = MatroskaTocFormat::new();
                {
                    let muxer = toc_setter_pipeline.get_muxer().unwrap();
                    let info = self
                        .playback_pipeline
                        .as_ref()
                        .unwrap()
                        .info
                        .read()
                        .unwrap();
                    exporter.export(&info, muxer);
                }

                toc_setter_pipeline
                    .export()
                    .map_err(|err| gettext("Failed to export media. {}").replacen("{}", &err, 1))?;

                self.toc_setter_pipeline = Some(toc_setter_pipeline);
                Ok(ProcessorStatus::InProgress)
            }
            MediaEvent::Eos => {
                Ok(ProcessorStatus::Completed(gettext("Media exported succesfully")))
            }
            MediaEvent::FailedToExport(err) => {
                Err(gettext("Failed to export media. {}").replacen("{}", &err, 1))
            }
            _ => Ok(ProcessorStatus::InProgress),
        }
    }
}

impl OutputUIController for ExportController {
    fn switch_to_busy(&self) {
        // TODO: allow cancelling export
        self.base.switch_to_busy();

        self.export_list.set_sensitive(false);
        self.export_btn.set_sensitive(false);
    }

    fn switch_to_available(&mut self) {
        self.base.switch_to_available();

        self.export_progress_bar.set_fraction(0f64);
        self.export_list.set_sensitive(true);
        self.export_btn.set_sensitive(true);
    }

    fn update_progress(&mut self) {
        if self.duration > 0 {
            let position = match self.toc_setter_pipeline.as_mut() {
                Some(toc_setter_pipeline) => toc_setter_pipeline.get_position(),
                None => 0,
            };
            self.export_progress_bar
                .set_fraction(position as f64 / self.duration as f64);
        }
    }
}

impl Deref for ExportController {
    type Target = OutputBaseController;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for ExportController {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
