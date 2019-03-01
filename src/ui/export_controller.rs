use gettextrs::gettext;
use glib;

use gtk;
use gtk::prelude::*;

use log::warn;

use std::{
    cell::RefCell,
    collections::HashSet,
    fs::File,
    ops::{Deref, DerefMut},
    path::Path,
    rc::{Rc, Weak},
    sync::mpsc::{channel, Receiver},
};

use crate::{
    media::{PipelineMessage, PipelineMessage::*, TocSetterPipeline},
    metadata,
    metadata::{Exporter, Format, MatroskaTocFormat},
};

use super::{MainController, OutputBaseController};

const LISTENER_PERIOD: u32 = 250; // 250 ms (4 Hz)

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

            this_mut.export_list.select_row(&this_mut.mkvmerge_txt_row);
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

        this.check_requirements();

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
                        this_clone.borrow_mut().export();
                        glib::Continue(false)
                    });
                }));
        });
    }

    pub fn new_media(&mut self) {
        self.export_btn.set_sensitive(true);
    }

    pub fn cleanup(&mut self) {
        self.export_btn.set_sensitive(false);
        self.export_progress_bar.set_fraction(0f64);
    }

    fn check_requirements(&self) {
        let _ = TocSetterPipeline::check_requirements().map_err(|err| {
            warn!("{}", err);
            self.mkvmerge_txt_warning_lbl.set_label(&err);
            self.mkv_row.set_sensitive(false);
        });
    }

    fn export(&mut self) {
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
                match File::create(&self.target_path) {
                    Ok(mut output_file) => {
                        let info = self
                            .playback_pipeline
                            .as_ref()
                            .unwrap()
                            .info
                            .read()
                            .unwrap();
                        match metadata::Factory::get_writer(format).write(&info, &mut output_file) {
                            Ok(_) => self.show_message(
                                gtk::MessageType::Info,
                                gettext("Table of contents exported succesfully"),
                            ),
                            Err(err) => self.show_message(gtk::MessageType::Error, err),
                        }
                    }
                    Err(_) => self.show_message(
                        gtk::MessageType::Error,
                        gettext("Failed to create the file for the table of contents"),
                    ),
                }

                self.restore_pipeline();
                self.switch_to_available();
            }
            ExportType::SingleFileWithToc => {
                let target_path = self.target_path.clone();
                self.build_pipeline(&target_path, stream_ids);
            }
        }
    }

    fn build_pipeline(&mut self, export_path: &Path, streams: HashSet<String>) {
        let (pipeline_tx, ui_rx) = channel();

        self.register_listener(LISTENER_PERIOD, ui_rx);

        match TocSetterPipeline::try_new(&self.media_path, export_path, streams, pipeline_tx) {
            Ok(toc_setter_pipeline) => {
                self.switch_to_busy();
                self.toc_setter_pipeline = Some(toc_setter_pipeline);
            }
            Err(error) => {
                self.remove_listener();
                self.switch_to_available();
                self.restore_pipeline();
                self.show_error(
                    gettext("Failed to prepare for export. {}").replacen("{}", &error, 1),
                );
            }
        };
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

    fn switch_to_busy(&self) {
        // TODO: allow cancelling export
        self.base.switch_to_busy();

        self.export_list.set_sensitive(false);
        self.export_btn.set_sensitive(false);
    }

    fn switch_to_available(&self) {
        self.base.switch_to_available();

        self.export_progress_bar.set_fraction(0f64);
        self.export_list.set_sensitive(true);
        self.export_btn.set_sensitive(true);
    }

    fn register_listener(&mut self, period: u32, ui_rx: Receiver<PipelineMessage>) {
        let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(period, move || {
            let mut keep_going = false;

            if let Some(this_rc) = this_weak.upgrade() {
                keep_going = true;
                let mut this = this_rc.borrow_mut();

                if this.duration > 0 {
                    let position = match this.toc_setter_pipeline.as_mut() {
                        Some(toc_setter_pipeline) => toc_setter_pipeline.get_position(),
                        None => 0,
                    };
                    this.export_progress_bar
                        .set_fraction(position as f64 / this.duration as f64);
                }

                for message in ui_rx.try_iter() {
                    match message {
                        InitDone => {
                            let mut toc_setter_pipeline = this.toc_setter_pipeline.take().unwrap();

                            let exporter = MatroskaTocFormat::new();
                            {
                                let muxer = toc_setter_pipeline.get_muxer().unwrap();
                                let info = this
                                    .playback_pipeline
                                    .as_ref()
                                    .unwrap()
                                    .info
                                    .read()
                                    .unwrap();
                                exporter.export(&info, muxer);
                            }

                            if let Err(err) = toc_setter_pipeline.export() {
                                keep_going = false;
                                this.show_error(
                                    gettext("Failed to export media. {}").replacen("{}", &err, 1),
                                );
                            }

                            this.toc_setter_pipeline = Some(toc_setter_pipeline);
                        }
                        Eos => {
                            this.show_info(gettext("Media exported succesfully"));
                            keep_going = false;
                        }
                        FailedToExport(error) => {
                            keep_going = false;
                            this.show_error(
                                gettext("Failed to export media. {}").replacen("{}", &error, 1),
                            );
                        }
                        _ => (),
                    };

                    if !keep_going {
                        break;
                    }
                }

                if !keep_going {
                    this.listener_src = None;
                    this.switch_to_available();
                    this.restore_pipeline();
                }
            }

            glib::Continue(keep_going)
        }));
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
