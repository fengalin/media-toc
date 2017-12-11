extern crate glib;

extern crate gtk;
use gtk::prelude::*;

use std::cell::RefCell;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};

use media::{ContextMessage, PlaybackContext, SplitterContext, TocSetterContext};
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
    toc_setter_ctx: Option<TocSetterContext>,
    splitter_ctx: Option<SplitterContext>,
    export_type: ExportType,
    media_path: PathBuf,
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
            toc_setter_ctx: None,
            splitter_ctx: None,
            export_type: ExportType::None,
            media_path: PathBuf::new(),
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
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let this = this_rc.borrow();

        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.export_dlg.connect_delete_event(move |dlg, _| {
            main_ctrl_clone.borrow_mut().restore_context(
                this_clone
                    .borrow_mut()
                    .playback_ctx
                    .take()
                    .unwrap(),
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
                this.playback_ctx
                    .as_ref()
                    .unwrap()
                    .info
                    .lock()
                    .expect(
                        "ExportController::export_btn clicked, failed to lock media info",
                    )
                    .video_best
                    .is_none()
            };
            this.extension = toc::Factory::get_extension(&format, is_audio_only).to_owned();

            this.media_path = this.playback_ctx.as_ref().unwrap().path.clone();
            this.target_path = this.media_path.with_extension(&this.extension);

            if this.listener_src.is_some() {
                this.remove_listener();
            }

            this.duration = this.playback_ctx.as_ref().unwrap().get_duration();

            match export_type {
                ExportType::ExternalToc => {
                    // export toc as a standalone file
                    // TODO: handle file related errors
                    let mut output_file = File::create(&this.target_path).expect(
                        "ExportController::export_btn clicked couldn't create output file",
                    );

                    {
                        let info = this.playback_ctx.as_ref().unwrap().info.lock().expect(
                            "ExportController::export_btn clicked, failed to lock media info",
                        );
                        toc::Factory::get_writer(&format).write(
                            &info.metadata,
                            &info.chapters,
                            &mut output_file,
                        );
                    }

                    main_ctrl_clone.borrow_mut().restore_context(
                        this.playback_ctx
                            .take()
                            .unwrap(),
                    );
                    this.export_dlg.hide();
                }
                ExportType::Split => {
                    let has_chapters = this.next_chapter().is_some();

                    if has_chapters {
                        this.build_splitter_context(&main_ctrl_clone);
                    } else {
                        // No chapter => export the whole file
                        let target_path = this.target_path.clone();
                        this.build_toc_setter_context(&main_ctrl_clone, &target_path);
                    }
                }
                ExportType::SingleFileWithToc => {
                    let target_path = this.target_path.clone();
                    this.build_toc_setter_context(&main_ctrl_clone, &target_path);
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

    fn build_toc_setter_context(
        &mut self,
        main_ctrl: &Rc<RefCell<MainController>>,
        export_path: &Path,
    ) {
        let (ctx_tx, ui_rx) = channel();

        self.register_toc_setter_listener(LISTENER_PERIOD, ui_rx, Rc::clone(main_ctrl));

        let media_path = self.media_path.clone();
        match TocSetterContext::new(&media_path, export_path, ctx_tx) {
            Ok(toc_setter_ctx) => {
                self.switch_to_busy();
                self.toc_setter_ctx = Some(toc_setter_ctx);
            }
            Err(error) => {
                eprintln!("Error exporting media: {}", error);
                self.remove_listener();
                self.switch_to_available();
                main_ctrl.borrow_mut().restore_context(
                    self.playback_ctx.take().unwrap(),
                );
                self.export_dlg.hide();
            }
        };
    }

    fn build_splitter_context(&mut self, main_ctrl: &Rc<RefCell<MainController>>) {
        let (ctx_tx, ui_rx) = channel();

        self.register_splitter_listener(LISTENER_PERIOD, ui_rx, Rc::clone(main_ctrl));

        let (output_path, start, end) = {
            let chapter = self.current_chapter.as_ref().expect(
                "ExportController::build_splitter_context no chapter",
            );
            (
                self.get_split_path(chapter),
                chapter.start.nano_total,
                chapter.end.nano_total,
            )
        };

        let media_path = self.media_path.clone();
        match SplitterContext::new(&media_path, &output_path, start, end, ctx_tx) {
            Ok(splitter_ctx) => {
                self.switch_to_busy();
                self.splitter_ctx = Some(splitter_ctx);
            }
            Err(error) => {
                eprintln!("Error exporting media: {}", error);
                self.remove_listener();
                self.switch_to_available();
                main_ctrl.borrow_mut().restore_context(
                    self.playback_ctx.take().unwrap(),
                );
                self.export_dlg.hide();
            }
        };
    }

    fn next_chapter(&mut self) -> Option<&Chapter> {
        let next_chapter = {
            let info = self.playback_ctx.as_ref().unwrap().info.lock().expect(
                "ExportController::next_chapter failed to lock media info",
            );

            info.chapters.get(self.idx).map(|chapter| {
                chapter.clone().to_owned()
            })
        };

        self.current_chapter = next_chapter;
        if self.current_chapter.is_some() {
            self.idx += 1;
        }

        self.current_chapter.as_ref()
    }

    fn get_split_path(&self, chapter: &Chapter) -> PathBuf {
        let mut split_name = String::new();

        let info = self.playback_ctx.as_ref().unwrap().info.lock().expect(
            "ExportController::get_split_name failed to lock media info",
        );

        // TODO: make format customisable
        if let Some(artist) = info.get_artist() {
            split_name += &format!("{} - ", artist);
        }
        if let Some(title) = info.get_title() {
            split_name += &format!("{} - ", title);
        }

        split_name += &format!(
                "{:02}. {}.{}",
                self.idx,
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
        self.split_rdbtn.set_sensitive(false);
    }

    fn switch_to_available(&self) {
        self.export_btn.set_sensitive(true);
        self.mkvmerge_txt_rdbtn.set_sensitive(true);
        self.cue_rdbtn.set_sensitive(true);
        self.mkv_rdbtn.set_sensitive(true);
        self.split_rdbtn.set_sensitive(true);
    }

    fn remove_listener(&mut self) {
        if let Some(source_id) = self.listener_src.take() {
            glib::source_remove(source_id);
        }
    }

    fn register_toc_setter_listener(
        &mut self,
        timeout: u32,
        ui_rx: Receiver<ContextMessage>,
        main_ctrl: Rc<RefCell<MainController>>,
    ) {
        let this_rc = Rc::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;

            let mut this = this_rc.borrow_mut();

            if this.duration > 0 {
                let position = match this.toc_setter_ctx.as_mut() {
                    Some(toc_setter_ctx) => toc_setter_ctx.get_position(),
                    None => 0,
                };
                this.progress_bar.set_fraction(
                    position as f64 / this.duration as f64,
                );
            }

            for message in ui_rx.try_iter() {
                match message {
                    AsyncDone => {
                        println!("ExportContext::toc_setter(AsyncDone)");
                    }
                    InitDone => {
                        let mut toc_setter_ctx = this.toc_setter_ctx.take().expect(
                            "ExportContext::toc_setter(InitDone) couldn't get ExportContext",
                        );

                        let exporter = MatroskaTocFormat::new();
                        {
                            let muxer = toc_setter_ctx.get_muxer().expect(
                                "ExportContext::toc_setter(InitDone) couldn't get muxer",
                            );

                            let info = this.playback_ctx.as_ref().unwrap().info.lock().expect(
                                concat!(
                                            "ExportController::toc_setter(InitDone) ",
                                            "failed to lock media info",
                                        ),
                            );
                            exporter.export(&info.metadata, &info.chapters, muxer);
                        }

                        match toc_setter_ctx.export() {
                            Ok(_) => (),
                            Err(err) => {
                                this.switch_to_available();
                                eprintln!("ERROR: failed to export media: {}", err);
                            }
                        }

                        this.toc_setter_ctx = Some(toc_setter_ctx);
                    }
                    Eos => {
                        this.switch_to_available();
                        main_ctrl.borrow_mut().restore_context(
                            this.playback_ctx.take().unwrap(),
                        );
                        this.export_dlg.hide();
                        keep_going = false;
                    }
                    FailedToExport => {
                        this.switch_to_available();
                        eprintln!("ERROR: failed to export media");
                        keep_going = false;
                    }
                    _ => (),
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

    fn register_splitter_listener(
        &mut self,
        timeout: u32,
        ui_rx: Receiver<ContextMessage>,
        main_ctrl: Rc<RefCell<MainController>>,
    ) {
        let this_rc = Rc::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;

            let mut this = this_rc.borrow_mut();

            if this.duration > 0 {
                let position = match this.splitter_ctx.as_mut() {
                    Some(splitter_ctx) => splitter_ctx.get_position(),
                    None => 0,
                };
                this.progress_bar.set_fraction(
                    position as f64 / this.duration as f64,
                );
            }

            for message in ui_rx.try_iter() {
                match message {
                    Eos => {
                        let has_more = this.next_chapter().is_some();
                        if has_more {
                            // build a new context for next chapter
                            this.build_splitter_context(&main_ctrl);
                        } else {
                            this.switch_to_available();
                            main_ctrl.borrow_mut().restore_context(
                                this.playback_ctx.take().unwrap(),
                            );
                            this.export_dlg.hide();
                        }

                        keep_going = false;
                    }
                    FailedToExport => {
                        this.switch_to_available();
                        eprintln!("ERROR: failed to export media");
                        keep_going = false;
                    }
                    _ => (),
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
