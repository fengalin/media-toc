use gettextrs::gettext;
use glib;

use gtk;
use gtk::prelude::*;

use log::{error, warn};

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
    media::{
        ContextMessage::*,
        ContextMessage, TocSetterContext,
    },
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

    toc_setter_ctx: Option<TocSetterContext>,
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

            toc_setter_ctx: None,
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
                .request_context(Box::new(move |context| {
                    {
                        this_clone.borrow_mut().playback_ctx = Some(context);
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
        let _ = TocSetterContext::check_requirements().map_err(|err| {
            warn!("{}", err);
            self.mkvmerge_txt_warning_lbl.set_label(&err);
            self.mkv_row.set_sensitive(false);
        });
    }

    fn export(&mut self) {
        debug_assert!(self.playback_ctx.is_some());
        let (format, export_type) = self.get_selection();

        match export_type {
            ExportType::ExternalToc => {
                self.prepare_process(format, false);
                // export toc as a standalone file
                let (msg_type, msg) = match File::create(&self.target_path) {
                    Ok(mut output_file) => {
                        let info = self.playback_ctx.as_ref().unwrap().info.read().unwrap();
                        match metadata::Factory::get_writer(format).write(&info, &mut output_file) {
                            Ok(_) => (
                                gtk::MessageType::Info,
                                gettext("Table of contents exported succesfully"),
                            ),
                            Err(err) => {
                                error!("{}", err);
                                (gtk::MessageType::Error, err)
                            }
                        }
                    }
                    Err(_) => {
                        let msg = gettext("Failed to create the file for the table of contents");
                        error!("{}", msg);
                        (gtk::MessageType::Error, msg)
                    }
                };

                self.restore_context();
                self.switch_to_available();
                self.show_message(msg_type, &msg);
            }
            ExportType::SingleFileWithToc => {
                let (streams, is_audio_only) = {
                    let mut has_audio = false;
                    let mut has_other = false;
                    let mut streams = HashSet::<String>::new();
                    let playback_ctx = self.playback_ctx.as_ref().unwrap();
                    let info = playback_ctx.info.read().unwrap();
                    for (ref stream_id, ref stream) in &info.streams.video {
                        if stream.must_export {
                            streams.insert(stream_id.to_string());
                            has_other = true;
                        }
                    }
                    for (ref stream_id, ref stream) in &info.streams.audio {
                        if stream.must_export {
                            streams.insert(stream_id.to_string());
                            has_audio = true;
                        }
                    }
                    for (ref stream_id, ref stream) in &info.streams.text {
                        if stream.must_export {
                            streams.insert(stream_id.to_string());
                            has_other = true;
                        }
                    }
                    (streams, has_audio && !has_other)
                };

                self.prepare_process(format, is_audio_only);
                let target_path = self.target_path.clone();
                self.build_context(&target_path, streams);
            }
        }
    }

    fn build_context(&mut self, export_path: &Path, streams: HashSet<String>) {
        let (ctx_tx, ui_rx) = channel();

        self.register_listener(LISTENER_PERIOD, ui_rx);

        match TocSetterContext::try_new(&self.media_path, export_path, streams, ctx_tx) {
            Ok(toc_setter_ctx) => {
                self.switch_to_busy();
                self.toc_setter_ctx = Some(toc_setter_ctx);
            }
            Err(error) => {
                self.remove_listener();
                self.switch_to_available();
                self.restore_context();
                let msg = gettext("Failed to prepare for export. {}").replacen("{}", &error, 1);
                self.show_error(&msg);
                error!("{}", msg);
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

    fn register_listener(&mut self, period: u32, ui_rx: Receiver<ContextMessage>) {
        let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(period, move || {
            let mut keep_going = false;

            if let Some(this_rc) = this_weak.upgrade() {
                keep_going = true;
                let mut this = this_rc.borrow_mut();

                if this.duration > 0 {
                    let position = match this.toc_setter_ctx.as_mut() {
                        Some(toc_setter_ctx) => toc_setter_ctx.get_position(),
                        None => 0,
                    };
                    this.export_progress_bar
                        .set_fraction(position as f64 / this.duration as f64);
                }

                for message in ui_rx.try_iter() {
                    match message {
                        InitDone => {
                            let mut toc_setter_ctx = this.toc_setter_ctx.take().unwrap();

                            let exporter = MatroskaTocFormat::new();
                            {
                                let muxer = toc_setter_ctx.get_muxer().unwrap();
                                let info = this.playback_ctx.as_ref().unwrap().info.read().unwrap();
                                exporter.export(&info, muxer);
                            }

                            if let Err(err) = toc_setter_ctx.export() {
                                keep_going = false;
                                let msg =
                                    gettext("Failed to export media. {}").replacen("{}", &err, 1);
                                this.show_error(&msg);
                                error!("{}", msg);
                            }

                            this.toc_setter_ctx = Some(toc_setter_ctx);
                        }
                        Eos => {
                            this.show_info(&gettext("Media exported succesfully"));
                            keep_going = false;
                        }
                        FailedToExport(error) => {
                            keep_going = false;
                            let message =
                                gettext("Failed to export media. {}").replacen("{}", &error, 1);
                            this.show_error(&message);
                            error!("{}", message);
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
                    this.restore_context();
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
