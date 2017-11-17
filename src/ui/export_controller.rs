extern crate glib;

extern crate gtk;
use gtk::prelude::*;

use std::fs::File;

use std::sync::mpsc::{channel, Receiver};

use std::rc::Rc;
use std::cell::RefCell;

use media::{ContextMessage, ExportContext, PlaybackContext};
use media::ContextMessage::*;

use toc;
use toc::{Exporter, MatroskaTocFormat};

use super::MainController;

const LISTENER_PERIOD: u32 = 250; // 250 ms (4 Hz)

pub struct ExportController {
    export_dlg: gtk::Dialog,
    export_btn: gtk::Button,

    mkvmerge_txt_rdbtn: gtk::RadioButton,
    cue_rdbtn: gtk::RadioButton,
    mkv_rdbtn: gtk::RadioButton,

    pub playback_ctx: Option<PlaybackContext>,
    export_ctx: Option<ExportContext>,

    this_opt: Option<Rc<RefCell<ExportController>>>,
    listener_src: Option<glib::SourceId>,
}

impl ExportController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let main_window: gtk::ApplicationWindow = builder.get_object("application-window").unwrap();
        let export_dlg: gtk::Dialog = builder.get_object("export-dlg").unwrap();
        export_dlg.set_transient_for(&main_window);

        let this = Rc::new(RefCell::new(ExportController {
            export_dlg: export_dlg,
            export_btn: builder.get_object("export-btn").unwrap(),

            mkvmerge_txt_rdbtn: builder.get_object("mkvmerge_txt-rdbtn").unwrap(),
            cue_rdbtn: builder.get_object("cue-rdbtn").unwrap(),
            mkv_rdbtn: builder.get_object("mkv-rdbtn").unwrap(),

            playback_ctx: None,
            export_ctx: None,

            this_opt: None,
            listener_src: None,
        }));

        {
            let mut this_mut = this.borrow_mut();
            let this_rc = Rc::clone(&this);
            this_mut.this_opt = Some(this_rc);
        }

        this
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>
    ) {
        let this = this_rc.borrow();

        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.export_dlg.connect_delete_event(move |dlg, _| {
            main_ctrl_clone.borrow_mut().restore_context(
                this_clone.borrow_mut().playback_ctx.take().unwrap()
            );
            dlg.hide_on_delete();
            Inhibit(true)
        });

        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.export_btn.connect_clicked(move |_| {
            let mut this = this_clone.borrow_mut();

            this.switch_to_available();

            let (format, is_standalone) = this.get_selected_format();

            let is_audio_only = {
                this.playback_ctx.as_ref().unwrap().info.lock()
                    .expect(
                        "ExportController::export_btn clicked, failed to lock media info",
                    ).video_best.is_none()
            };
            let extension = toc::Factory::get_extension(&format, is_audio_only);

            let media_path = this.playback_ctx.as_ref().unwrap().path.clone();
            let target_path = media_path.with_file_name(&format!("{}.{}",
                media_path.file_stem()
                    .expect("ExportController::export_btn clicked, failed to get file_stem")
                    .to_str()
                    .expect("ExportController::export_btn clicked, failed to get file_stem as str"),
                extension,
            ));

            if is_standalone {
                // export toc as a standalone file
                // TODO: handle file related errors
                let mut output_file = File::create(target_path)
                    .expect("ExportController::export_btn clicked couldn't create output file");

                {
                    let info = this.playback_ctx.as_ref().unwrap().info.lock()
                        .expect(
                            "ExportController::export_btn clicked, failed to lock media info",
                        );
                    toc::Factory::get_writer(&format)
                        .write(&info.metadata, &info.chapters, &mut output_file);
                }

                main_ctrl_clone.borrow_mut()
                    .restore_context(this.playback_ctx.take().unwrap());
                this.export_dlg.hide();
            } else {
                // export toc within a media container with the streams
                let (ctx_tx, ui_rx) = channel();

                this.register_listener(LISTENER_PERIOD, ui_rx, main_ctrl_clone.clone());

                match ExportContext::new(media_path, target_path, ctx_tx) {
                    Ok(export_ctx) => {
                        this.switch_to_busy();
                        this.export_ctx = Some(export_ctx);
                        println!("Exporting...");
                    },
                    Err(error) => {
                        eprintln!("Error exporting media: {}", error);
                        this.remove_listener();
                        this.switch_to_available();
                        main_ctrl_clone.borrow_mut()
                            .restore_context(this.playback_ctx.take().unwrap());
                        this.export_dlg.hide();
                    }
                };
            }
        });
    }

    pub fn open(&mut self, playback_ctx: PlaybackContext) {
        self.playback_ctx = Some(playback_ctx);
        self.export_dlg.present();
    }

    fn get_selected_format(&self) -> (toc::Format, bool) {
        if self.mkvmerge_txt_rdbtn.get_active() {
            (toc::Format::MKVMergeText, true)
        } else if self.cue_rdbtn.get_active() {
            (toc::Format::CueSheet, true)
        } else if self.mkv_rdbtn.get_active() {
            (toc::Format::Matroska, false)
        } else {
            unreachable!("ExportController::get_selected_format no selected radio button");
        }
    }

    fn switch_to_busy(&self) {
        self.export_btn.set_sensitive(false);
        self.mkvmerge_txt_rdbtn.set_sensitive(false);
        self.cue_rdbtn.set_sensitive(false);
        self.mkv_rdbtn.set_sensitive(false);
    }

    fn switch_to_available(&self) {
        self.export_btn.set_sensitive(true);
        self.mkvmerge_txt_rdbtn.set_sensitive(true);
        self.cue_rdbtn.set_sensitive(true);
        self.mkv_rdbtn.set_sensitive(true);
    }

    fn remove_listener(&mut self) {
        if let Some(source_id) = self.listener_src.take() {
            glib::source_remove(source_id);
        }
    }

    fn register_listener(&mut self,
        timeout: u32,
        ui_rx: Receiver<ContextMessage>,
        main_ctrl: Rc<RefCell<MainController>>,
    ) {
        if self.listener_src.is_some() {
            return;
        }

        let this_rc = Rc::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;

            for message in ui_rx.try_iter() {
                match message {
                    AsyncDone => {
                        println!("ExportContext::listener(AsyncDone)");
                    }
                    InitDone => {
                        let this = this_rc.borrow();
                        let export_ctx = this.export_ctx.as_ref()
                            .expect("ExportContext::listener(InitDone) couldn't get ExportContext");

                        match export_ctx.get_muxer() {
                            Some(muxer) => {
                                let info = this.playback_ctx.as_ref().unwrap()
                                    .info
                                        .lock()
                                            .expect(concat!(
                                                "ExportController::listener(InitDone) ",
                                                "failed to lock media info",
                                            ));

                                let exporter = MatroskaTocFormat::new();
                                exporter.export(&info.metadata, &info.chapters, &muxer);

                                match export_ctx.export() {
                                    Ok(_) => (),
                                    Err(err) => {
                                        this.switch_to_available();
                                        eprintln!("ERROR: failed to export media: {}", err);
                                    }
                                }
                            }
                            None => {
                                this.switch_to_available();
                                eprintln!("ExportContext::listener(InitDone) couldn't get ")
                            }
                        }
                    }
                    Eos => {
                        let mut this = this_rc.borrow_mut();
                        this.switch_to_available();
                        main_ctrl.borrow_mut()
                            .restore_context(this.playback_ctx.take().unwrap());
                        this.export_dlg.hide();
                        println!("Done");
                        keep_going = false;
                    }
                    FailedToOpenMedia => {
                        this_rc.borrow()
                            .switch_to_available();
                        eprintln!("ERROR: failed to export media");
                        keep_going = false;
                    }
                };

                if !keep_going {
                    break;
                }
            }

            if !keep_going {
                let mut this = this_rc.borrow_mut();
                this.listener_src = None;
            }

            glib::Continue(keep_going)
        }));
    }
}
