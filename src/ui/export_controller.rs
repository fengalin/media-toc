extern crate glib;

extern crate gtk;
use gtk::prelude::*;

use std::cell::RefCell;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};

use media::{ContextMessage, ExportContext, PlaybackContext};
use media::ContextMessage::*;

use toc;
use toc::{Chapter, DEFAULT_TITLE, Exporter, MatroskaTocFormat};

use super::MainController;

const LISTENER_PERIOD: u32 = 250; // 250 ms (4 Hz)

#[derive(Clone, PartialEq)]
enum ExportType {
    ExternalToc,
    None,
    SingleFileWithToc,
    Split,
}

pub struct ExportController {
    export_dlg: gtk::Dialog,
    export_btn: gtk::Button,
    progress_bar: gtk::ProgressBar,

    mkvmerge_txt_rdbtn: gtk::RadioButton,
    cue_rdbtn: gtk::RadioButton,
    mkv_rdbtn: gtk::RadioButton,
    split_rdbtn: gtk::RadioButton,

    pub playback_ctx: Option<PlaybackContext>,
    export_ctx: Option<ExportContext>,
    export_type: ExportType,
    target_path: PathBuf,
    extension: String,
    idx: usize,
    current_chapter: Option<Chapter>,
    duration: u64,

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
            progress_bar: builder.get_object("export-progress").unwrap(),

            mkvmerge_txt_rdbtn: builder.get_object("mkvmerge_txt-rdbtn").unwrap(),
            cue_rdbtn: builder.get_object("cue-rdbtn").unwrap(),
            mkv_rdbtn: builder.get_object("mkv-rdbtn").unwrap(),
            split_rdbtn: builder.get_object("split-rdbtn").unwrap(),

            playback_ctx: None,
            export_ctx: None,
            export_type: ExportType::None,
            target_path: PathBuf::new(),
            extension: String::new(),
            idx: 0,
            current_chapter: None,
            duration: 0,

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

            let (format, export_type) = this.get_selected_format();
            this.export_type = export_type.clone();
            this.idx = 0;

            let is_audio_only = {
                this.playback_ctx.as_ref().unwrap().info.lock()
                    .expect(
                        "ExportController::export_btn clicked, failed to lock media info",
                    ).video_best.is_none()
            };
            this.extension = toc::Factory::get_extension(&format, is_audio_only).to_owned();

            let media_path = this.playback_ctx.as_ref().unwrap().path.clone();
            this.target_path = media_path.with_extension(&this.extension);

            match export_type {
                ExportType::ExternalToc => {
                    // export toc as a standalone file
                    // TODO: handle file related errors
                    let mut output_file = File::create(&this.target_path)
                        .expect("ExportController::export_btn clicked couldn't create output file");

                    {
                        let info = this.playback_ctx.as_ref()
                            .unwrap().info.lock()
                            .expect(
                                "ExportController::export_btn clicked, failed to lock media info",
                            );
                        toc::Factory::get_writer(&format)
                            .write(&info.metadata, &info.chapters, &mut output_file);
                    }

                    main_ctrl_clone.borrow_mut()
                        .restore_context(this.playback_ctx.take().unwrap());
                    this.export_dlg.hide();
                }
                ExportType::Split => {
                    this.next_chapter();

                    let first_path =
                        match this.current_chapter {
                            Some(ref chapter) => this.get_split_path(chapter),
                            None => // No chapter => export the whole file
                                this.target_path.clone(),
                        };
                    this.build_export_context(
                        Rc::clone(&main_ctrl_clone),
                        &media_path,
                        &first_path,
                    );
                }
                ExportType::SingleFileWithToc => {
                    let target_path = this.target_path.clone();
                    this.build_export_context(
                        Rc::clone(&main_ctrl_clone),
                        &media_path,
                        &target_path,
                    );
                }
                _ => (),
            }
        });
    }

    pub fn open(&mut self, playback_ctx: PlaybackContext) {
        self.playback_ctx = Some(playback_ctx);
        self.progress_bar.set_fraction(0f64);
        self.export_dlg.present();
    }

    fn build_export_context(&mut self,
        main_ctrl: Rc<RefCell<MainController>>,
        media_path: &Path,
        export_path: &Path,
    ) {
        self.duration = self.playback_ctx.as_ref()
            .unwrap()
            .get_duration();
        // export toc within a media container with the streams
        let (ctx_tx, ui_rx) = channel();

        self.register_listener(LISTENER_PERIOD, ui_rx, Rc::clone(&main_ctrl));

        match ExportContext::new(media_path, export_path, ctx_tx) {
            Ok(export_ctx) => {
                self.switch_to_busy();
                self.export_ctx = Some(export_ctx);
            },
            Err(error) => {
                eprintln!("Error exporting media: {}", error);
                self.remove_listener();
                self.switch_to_available();
                main_ctrl.borrow_mut()
                    .restore_context(self.playback_ctx.take().unwrap());
                self.export_dlg.hide();
            }
        };
    }

    fn next_chapter(&mut self) {
        let next_chapter = {
            let info = self.playback_ctx.as_ref().unwrap()
                .info.lock()
                    .expect("ExportController::next_chapter failed to lock media info");

            info.chapters.get(self.idx)
                .map(|chapter| chapter.clone().to_owned())
        };

        self.current_chapter = next_chapter;
        if let Some(_) = self.current_chapter {
            self.idx += 1;
        }
    }

    fn get_split_path(&self, chapter: &Chapter) -> PathBuf {
        let mut split_name = String::new();

        let info = self.playback_ctx.as_ref().unwrap()
            .info.lock()
                .expect("ExportController::get_split_name failed to lock media info");

        // TODO: make format customisable
        if let Some(artist) = info.get_artist() {
            split_name += &format!("{} - ", artist);
        }
        if let Some(title) = info.get_title() {
            split_name += &format!("{} - ", title);
        }

        split_name +=
            &format!(
                "{:02}. {}.{}",
                self.idx + 1,
                chapter.get_title().unwrap_or(DEFAULT_TITLE),
                self.extension,
            );

        self.target_path.with_file_name(split_name)
    }

    fn get_selected_format(&self) -> (toc::Format, ExportType) {
        if self.mkvmerge_txt_rdbtn.get_active() {
            (toc::Format::MKVMergeText, ExportType::ExternalToc)
        } else if self.cue_rdbtn.get_active() {
            (toc::Format::CueSheet, ExportType::ExternalToc)
        } else if self.mkv_rdbtn.get_active() {
            (toc::Format::Matroska, ExportType::SingleFileWithToc)
        } else if self.split_rdbtn.get_active() {
            (toc::Format::Matroska, ExportType::Split)
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

            let mut this = this_rc.borrow_mut();

            if this.duration > 0 {
                let position =
                    match this.export_ctx.as_mut() {
                        Some(export_ctx) => export_ctx.get_position(),
                        None => 0,
                    };
                this.progress_bar.set_fraction(position as f64 / this.duration as f64);
            }

            for message in ui_rx.try_iter() {
                match message {
                    AsyncDone => {
                        println!("ExportContext::listener(AsyncDone)");
                    }
                    InitDone => {
                        let export_ctx = this.export_ctx.as_ref()
                            .expect("ExportContext::listener(InitDone) couldn't get ExportContext");

                        match export_ctx.get_muxer() {
                            Some(muxer) => {
                                match this.export_type {
                                    ExportType::Split => {
                                        match this.current_chapter {
                                            Some(ref chapter) => {
                                                println!("Split chapter found");
                                                let target_path = this.get_split_path(chapter);
                                                println!("target_path {:?}", target_path);
                                                match export_ctx.export_part(
                                                    &target_path,
                                                    chapter.start.nano_total,
                                                    chapter.end.nano_total,
                                                ) {
                                                    Ok(_) => println!("export part"),
                                                    Err(_) => {
                                                        this.switch_to_available();
                                                        eprintln!(
                                                            "ERROR: failed to export part {:?}",
                                                            target_path,
                                                        );
                                                    }
                                                }
                                            }
                                            None => {
                                                println!("Split no chapter");
                                                match export_ctx.export() {
                                                    Ok(_) => (),
                                                    Err(err) => {
                                                        this.switch_to_available();
                                                        eprintln!(
                                                            "ERROR: failed to export media: {}",
                                                            err,
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    ExportType::SingleFileWithToc => {
                                        let exporter = MatroskaTocFormat::new();
                                        {
                                            let info = this.playback_ctx.as_ref().unwrap()
                                                .info
                                                    .lock()
                                                        .expect(concat!(
                                                            "ExportController::listener(InitDone) ",
                                                            "failed to lock media info",
                                                        ));
                                            exporter.export(&info.metadata, &info.chapters, &muxer);
                                        }

                                        match export_ctx.export() {
                                            Ok(_) => (),
                                            Err(err) => {
                                                this.switch_to_available();
                                                eprintln!("ERROR: failed to export media: {}", err);
                                            }
                                        }
                                    }
                                    _ => unreachable!(concat!(
                                        "ExportController::listener(InitDone) ",
                                        "Unexpected export type",
                                    )),
                                }
                            }
                            None => {
                                this.switch_to_available();
                                eprintln!("ExportContext::listener(InitDone) couldn't get ")
                            }
                        }
                    }
                    Eos => {
                        this.switch_to_available();
                        main_ctrl.borrow_mut()
                            .restore_context(this.playback_ctx.take().unwrap());
                        this.export_dlg.hide();
                        keep_going = false;
                    }
                    FailedToOpenMedia => {
                        this.switch_to_available();
                        eprintln!("ERROR: failed to export media");
                        keep_going = false;
                    }
                };

                if !keep_going {
                    break;
                }
            }

            if !keep_going {
                this.listener_src = None;
            }

            glib::Continue(keep_going)
        }));
    }
}
