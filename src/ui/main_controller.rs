extern crate gdk;
extern crate glib;
extern crate gstreamer as gst;
extern crate gtk;

use std::rc::Rc;
use std::cell::RefCell;

use std::path::PathBuf;

use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};

use gdk::{Cursor, CursorType, WindowExt};

use gtk::prelude::*;

use media::{ContextMessage, PlaybackContext};
use media::ContextMessage::*;

use super::{AudioController, ExportController, InfoController, VideoController};

#[derive(Clone, Debug, PartialEq)]
pub enum ControllerState {
    EOS,
    Ready,
    Paused,
    PendingExportToc,
    PendingSelectMedia,
    Playing,
    PlayingRange(u64),
    Stopped,
    Seeking(bool, bool), // (must_switch_to_play, must_keep_paused)
}

const LISTENER_PERIOD: u32 = 250; // 250 ms (4 Hz)

pub struct MainController {
    window: gtk::ApplicationWindow,
    header_bar: gtk::HeaderBar,
    play_pause_btn: gtk::ToolButton,
    export_toc_btn: gtk::Button,

    video_ctrl: VideoController,
    info_ctrl: Rc<RefCell<InfoController>>,
    audio_ctrl: Rc<RefCell<AudioController>>,
    export_ctrl: Rc<RefCell<ExportController>>,

    context: Option<PlaybackContext>,
    state: ControllerState,

    requires_async_dialog: bool, // when pipeline contains video, dialogs must wait for
    // asyncdone before opening a dialog otherwise the listener
    // may borrow the MainController while the dialog is already
    // using it leading to a borrowing conflict
    this_opt: Option<Rc<RefCell<MainController>>>,
    keep_going: bool,
    listener_src: Option<glib::SourceId>,
}

impl MainController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let export_toc_btn: gtk::Button = builder.get_object("export_toc-btn").unwrap();
        export_toc_btn.set_sensitive(false);

        let this = Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").unwrap(),
            header_bar: builder.get_object("header-bar").unwrap(),
            play_pause_btn: builder.get_object("play_pause-toolbutton").unwrap(),
            export_toc_btn: export_toc_btn,

            video_ctrl: VideoController::new(builder),
            info_ctrl: InfoController::new(builder),
            audio_ctrl: AudioController::new(builder),
            export_ctrl: ExportController::new(builder),

            context: None,
            state: ControllerState::Stopped,

            requires_async_dialog: false,

            this_opt: None,
            keep_going: true,
            listener_src: None,
        }));

        {
            let mut this_mut = this.borrow_mut();

            let this_rc = Rc::clone(&this);
            this_mut.this_opt = Some(this_rc);

            this_mut.window.connect_delete_event(|_, _| {
                gtk::main_quit();
                Inhibit(false)
            });
            this_mut.window.set_titlebar(&this_mut.header_bar);

            let this_rc = Rc::clone(&this);
            this_mut.play_pause_btn.connect_clicked(move |_| {
                this_rc.borrow_mut().play_pause();
            });

            // TODO: add key bindings to seek by steps
            // play/pause, etc.

            let this_rc = Rc::clone(&this);
            this_mut.export_toc_btn.connect_clicked(move |_| {
                let mut this = this_rc.borrow_mut();

                if this.requires_async_dialog && this.state == ControllerState::Playing {
                    this.hold();
                    this.state = ControllerState::PendingExportToc;
                } else {
                    this.hold();
                    this.export_toc();
                }
            });

            this_mut.video_ctrl.register_callbacks(&this);
            InfoController::register_callbacks(&this_mut.info_ctrl, &this);
            AudioController::register_callbacks(&this_mut.audio_ctrl, &this);
            ExportController::register_callbacks(&this_mut.export_ctrl, &this);
        }

        let open_btn: gtk::Button = builder.get_object("open-btn").unwrap();
        let this_rc = Rc::clone(&this);
        open_btn.connect_clicked(move |_| {
            let mut this = this_rc.borrow_mut();

            if this.requires_async_dialog && this.state == ControllerState::Playing {
                this.hold();
                this.state = ControllerState::PendingSelectMedia;
            } else {
                this.hold();
                this.select_media();
            }
        });

        this
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }

    pub fn get_state(&self) -> &ControllerState {
        &self.state
    }

    pub fn play_pause(&mut self) {
        let mut context = match self.context.take() {
            Some(context) => context,
            None => {
                self.select_media();
                return;
            }
        };

        if self.state != ControllerState::EOS {
            match context.get_state() {
                gst::State::Paused => {
                    AudioController::register_tick_callback(&self.audio_ctrl);
                    self.play_pause_btn.set_icon_name("media-playback-pause");
                    self.state = ControllerState::Playing;
                    context.play().unwrap();
                    self.context = Some(context);
                }
                gst::State::Playing => {
                    context.pause().unwrap();
                    self.play_pause_btn.set_icon_name("media-playback-start");
                    AudioController::remove_tick_callback(&self.audio_ctrl);
                    self.state = ControllerState::Paused;
                    self.context = Some(context);
                }
                _ => {
                    self.context = Some(context);
                    self.select_media();
                }
            };
        } else {
            // Restart the stream from the begining
            self.context = Some(context);
            self.seek(0, true); // accurate (slow)
        }
    }

    pub fn seek(&mut self, position: u64, accurate: bool) {
        match self.state {
            ControllerState::Stopped | ControllerState::PlayingRange(_) => (),
            _ => {
                if self.state == ControllerState::Playing || self.state == ControllerState::Paused {
                    self.info_ctrl.borrow_mut().seek(position, &self.state);
                    self.audio_ctrl.borrow_mut().seek(position, &self.state);
                }

                let (must_switch_to_play, must_keep_paused) = match self.state {
                    ControllerState::EOS
                    | ControllerState::Ready
                    | ControllerState::Seeking(true, false) => (true, false),
                    ControllerState::Seeking(true, true) => (true, true),
                    ControllerState::Paused | ControllerState::Seeking(false, true) => {
                        (false, true)
                    }
                    _ => (false, false),
                };
                self.state = ControllerState::Seeking(must_switch_to_play, must_keep_paused);

                self.context
                    .as_ref()
                    .expect("MainController::seek no context")
                    .seek(position, accurate);
            }
        }
    }

    pub fn play_range(&mut self, start: u64, end: u64, pos_to_restore: u64) {
        if self.state == ControllerState::Paused {
            self.info_ctrl.borrow_mut().start_play_range();
            self.audio_ctrl.borrow_mut().start_play_range();

            self.state = ControllerState::PlayingRange(pos_to_restore);

            self.context
                .as_ref()
                .expect("MainController::play_range no context")
                .seek_range(start, end);

            AudioController::register_tick_callback(&self.audio_ctrl);
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.context
            .as_mut()
            .expect("MainController::get_position no context")
            .get_position()
    }

    pub fn refresh_info(&mut self, position: u64) {
        self.info_ctrl.borrow_mut().tick(position, false);
    }

    fn hold(&mut self) {
        AudioController::remove_tick_callback(&self.audio_ctrl);
        self.switch_to_busy();

        if let Some(context) = self.context.as_mut() {
            context.pause().unwrap();
        };

        self.play_pause_btn.set_icon_name("media-playback-start");
    }

    fn export_toc(&mut self) {
        if let Some(mut context) = self.context.take() {
            self.info_ctrl.borrow().export_chapters(&mut context);
            self.export_ctrl.borrow_mut().open(context);
            self.state = ControllerState::Paused;
        }
    }

    pub fn set_context(&mut self, context: PlaybackContext) {
        self.context = Some(context);
        self.state = ControllerState::Paused;
        self.switch_to_default();
    }

    fn remove_listener(&mut self) {
        if let Some(source_id) = self.listener_src.take() {
            glib::source_remove(source_id);
        }
    }

    fn register_listener(&mut self, timeout: u32, ui_rx: Receiver<ContextMessage>) {
        if self.listener_src.is_some() {
            return;
        }

        let this_rc = Rc::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;

            for message in ui_rx.try_iter() {
                match message {
                    AsyncDone => {
                        let mut this = this_rc.borrow_mut();
                        match this.state {
                            ControllerState::PendingSelectMedia => this.select_media(),
                            ControllerState::PendingExportToc => this.export_toc(),
                            ControllerState::Seeking(must_switch_to_play, must_keep_paused) => {
                                if must_switch_to_play {
                                    this.context
                                        .as_mut()
                                        .expect("MainController::listener(AsyncDone) no context")
                                        .play()
                                        .unwrap();
                                    AudioController::register_tick_callback(&this.audio_ctrl);
                                    this.play_pause_btn.set_icon_name("media-playback-pause");
                                    this.state = ControllerState::Playing;
                                } else if must_keep_paused {
                                    this.state = ControllerState::Paused;
                                } else {
                                    this.state = ControllerState::Playing;
                                }
                            }
                            _ => (),
                        }
                    }
                    InitDone => {
                        let mut this = this_rc.borrow_mut();
                        let context = this.context
                            .take()
                            .expect("MainController: InitDone but no context available");

                        this.requires_async_dialog = context
                            .info
                            .lock()
                            .expect("MainController::listener(InitDone) failed to lock media info")
                            .video_best
                            .is_some();

                        this.header_bar
                            .set_subtitle(Some(context.file_name.as_str()));

                        this.video_ctrl.new_media(&context);
                        this.info_ctrl.borrow_mut().new_media(&context);
                        this.audio_ctrl.borrow_mut().new_media(&context);

                        this.set_context(context);
                        this.export_toc_btn.set_sensitive(true);

                        this.state = ControllerState::Ready;
                    }
                    Eos => {
                        let mut this = this_rc.borrow_mut();
                        match this.state {
                            ControllerState::PlayingRange(pos_to_restore) => {
                                // end of range => pause and seek back to pos_to_restore
                                this.context
                                    .as_ref()
                                    .expect("MainController::listener(eos) no context")
                                    .pause()
                                    .unwrap();
                                this.state = ControllerState::Paused;
                                AudioController::remove_tick_callback(&this.audio_ctrl);
                                this.seek(pos_to_restore, true); // accurate
                            }
                            _ => {
                                #[cfg(feature = "trace-main-controller")]
                                println!("MainController::listener(eos)");

                                this.play_pause_btn.set_icon_name("media-playback-start");
                                this.state = ControllerState::EOS;

                                // The tick callback will be register again in case of a seek
                                AudioController::remove_tick_callback(&this.audio_ctrl);
                            }
                        }
                    }
                    FailedToOpenMedia => {
                        eprintln!("ERROR: failed to open media");

                        let mut this = this_rc.borrow_mut();
                        this.context = None;
                        this.state = ControllerState::Stopped;
                        this.switch_to_default();

                        this.keep_going = false;
                        keep_going = false;
                    }
                    _ => (),
                };

                if !keep_going {
                    break;
                }
            }

            if !keep_going {
                let mut this = this_rc.borrow_mut();
                this.remove_listener();
                AudioController::remove_tick_callback(&this.audio_ctrl);
            }

            glib::Continue(keep_going)
        }));
    }

    fn switch_to_busy(&mut self) {
        self.window.set_sensitive(false);

        let gdk_window = self.window.get_window().unwrap();
        gdk_window.set_cursor(
            &Cursor::new_for_display(
                &gdk_window.get_display(),
                CursorType::Watch,
            )
        );
    }

    fn switch_to_default(&mut self) {
        self.window.get_window()
            .unwrap()
            .set_cursor(None);
        self.window.set_sensitive(true);
    }

    fn select_media(&mut self) {
        self.switch_to_busy();

        let file_dlg = gtk::FileChooserDialog::new(
            Some("Open a media file"),
            Some(&self.window),
            gtk::FileChooserAction::Open,
        );
        // Note: couldn't find equivalents for STOCK_OK
        file_dlg.add_button("Open", gtk::ResponseType::Ok.into());

        if file_dlg.run() == gtk::ResponseType::Ok.into() {
            if let Some(ref context) = self.context {
                context.stop();
            }
            self.open_media(file_dlg.get_filename().unwrap());
        } else {
            if self.context.is_some() {
                self.state = ControllerState::Paused;
            }
            self.switch_to_default();
        }

        file_dlg.close();
    }

    fn open_media(&mut self, filepath: PathBuf) {
        self.remove_listener();

        self.info_ctrl.borrow_mut().cleanup();
        self.audio_ctrl.borrow_mut().cleanup();
        self.header_bar.set_subtitle("");
        self.export_toc_btn.set_sensitive(false);

        let (ctx_tx, ui_rx) = channel();

        self.keep_going = true;
        self.register_listener(LISTENER_PERIOD, ui_rx);

        let dbl_buffer_mtx = Arc::clone(&self.audio_ctrl.borrow().get_dbl_buffer_mtx());
        match PlaybackContext::new(filepath, dbl_buffer_mtx, ctx_tx) {
            Ok(context) => {
                self.context = Some(context);
            }
            Err(error) => {
                self.switch_to_default();
                eprintln!("Error opening media: {}", error);
            }
        };
    }
}
