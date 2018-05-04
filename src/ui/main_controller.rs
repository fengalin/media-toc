use std::rc::Rc;
use std::cell::RefCell;

use std::collections::HashSet;

use std::path::PathBuf;

use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};

use gettextrs::{gettext, ngettext};
use glib;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use gdk::{Cursor, CursorType, WindowExt};

use media::{ContextMessage, PlaybackContext};
use media::ContextMessage::*;

use super::{AudioController, ChaptersBoundaries, ExportController, InfoController,
            PerspectiveController, StreamsController, SplitController, VideoController};

const PAUSE_ICON: &str = "media-playback-pause-symbolic";
const PLAYBACK_ICON: &str = "media-playback-start-symbolic";

#[derive(PartialEq)]
pub enum ControllerState {
    EOS,
    Paused,
    PendingTakeContext,
    PendingSelectMedia,
    Playing,
    PlayingRange(u64),
    Ready,
    Seeking {
        seek_pos: u64,
        switch_to_play: bool,
        keep_paused: bool,
    },
    Stopped,
    TwoStepsSeek(u64),
}

const LISTENER_PERIOD: u32 = 100; // 100 ms (10 Hz)

pub struct MainController {
    window: gtk::ApplicationWindow,
    header_bar: gtk::HeaderBar,
    open_btn: gtk::Button,
    play_pause_btn: gtk::ToolButton,
    info_bar: gtk::InfoBar,
    info_bar_lbl: gtk::Label,

    perspective_ctrl: Rc<RefCell<PerspectiveController>>,
    video_ctrl: VideoController,
    info_ctrl: Rc<RefCell<InfoController>>,
    audio_ctrl: Rc<RefCell<AudioController>>,
    export_ctrl: Rc<RefCell<ExportController>>,
    split_ctrl: Rc<RefCell<SplitController>>,
    streams_ctrl: Rc<RefCell<StreamsController>>,

    pub context: Option<PlaybackContext>,
    take_context_cb: Option<Box<FnMut(PlaybackContext)>>,
    missing_plugins: HashSet<String>,
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
    pub fn new(builder: &gtk::Builder, is_gst_ok: bool) -> Rc<RefCell<Self>> {
        let chapters_boundaries = Rc::new(RefCell::new(ChaptersBoundaries::new()));

        let this = Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").unwrap(),
            header_bar: builder.get_object("header-bar").unwrap(),
            open_btn: builder.get_object("open-btn").unwrap(),
            play_pause_btn: builder.get_object("play_pause-toolbutton").unwrap(),
            info_bar: builder.get_object("info-bar").unwrap(),
            info_bar_lbl: builder.get_object("info_bar-lbl").unwrap(),

            perspective_ctrl: PerspectiveController::new(builder),
            video_ctrl: VideoController::new(builder),
            info_ctrl: InfoController::new(builder, Rc::clone(&chapters_boundaries)),
            audio_ctrl: AudioController::new(builder, chapters_boundaries),
            export_ctrl: ExportController::new(builder),
            split_ctrl: SplitController::new(builder),
            streams_ctrl: StreamsController::new(builder),

            context: None,
            take_context_cb: None,
            missing_plugins: HashSet::<String>::new(),
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

            this_mut
                .info_bar
                .connect_response(|info_bar, _| info_bar.hide());

            if is_gst_ok {
                this_mut.video_ctrl.register_callbacks(&this);
                PerspectiveController::register_callbacks(&this_mut.perspective_ctrl, &this);
                InfoController::register_callbacks(&this_mut.info_ctrl, &this);
                AudioController::register_callbacks(&this_mut.audio_ctrl, &this);
                ExportController::register_callbacks(&this_mut.export_ctrl, &this);
                SplitController::register_callbacks(&this_mut.split_ctrl, &this);
                StreamsController::register_callbacks(&this_mut.streams_ctrl, &this);

                let _ = PlaybackContext::check_requirements()
                    .map_err(|err| {
                        error!("{}", err);
                        let this_rc = Rc::clone(&this);
                        gtk::idle_add(move || {
                            this_rc
                                .borrow()
                                .show_message(gtk::MessageType::Warning, &err);
                            glib::Continue(false)
                        });
                    });

                let this_rc = Rc::clone(&this);
                this_mut.open_btn.connect_clicked(move |_| {
                    let mut this = this_rc.borrow_mut();

                    if this.requires_async_dialog && this.state == ControllerState::Playing {
                        this.hold();
                        this.state = ControllerState::PendingSelectMedia;
                    } else {
                        this.hold();
                        this.select_media();
                    }
                });
                this_mut.open_btn.set_sensitive(true);

                let this_rc = Rc::clone(&this);
                this_mut.play_pause_btn.connect_clicked(move |_| {
                    this_rc.borrow_mut().play_pause();
                });
                this_mut.play_pause_btn.set_sensitive(true);

            // TODO: add key bindings to seek by steps
            // play/pause, etc.
            } else {
                // GStreamer initialization failed
                this_mut.info_bar.connect_response(|_, _| gtk::main_quit());

                let msg = gettext("Failed to initialize GStreamer, the application can't be used.");
                this_mut.show_message(gtk::MessageType::Error, &msg);
                error!("{}", msg);
            }
        }

        this
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }

    pub fn show_message(&self, type_: gtk::MessageType, message: &str) {
        self.info_bar.set_message_type(type_);
        self.info_bar_lbl.set_label(message);
        self.info_bar.show();
        // workaround, see: https://bugzilla.gnome.org/show_bug.cgi?id=710888
        self.info_bar.queue_resize();
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
                    self.play_pause_btn.set_icon_name(PAUSE_ICON);
                    self.state = ControllerState::Playing;
                    self.audio_ctrl.borrow_mut().switch_to_playing();
                    context.play().unwrap();
                    self.context = Some(context);
                }
                gst::State::Playing => {
                    context.pause().unwrap();
                    self.play_pause_btn.set_icon_name(PLAYBACK_ICON);
                    self.state = ControllerState::Paused;
                    self.audio_ctrl.borrow_mut().switch_to_not_playing();
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

    pub fn move_chapter_boundary(&mut self, boundary: u64, to_position: u64) -> bool {
        self.info_ctrl
            .borrow_mut()
            .move_chapter_boundary(boundary, to_position)
    }

    pub fn seek(&mut self, position: u64, accurate: bool) {
        let mut must_sync_ctrl = false;
        let mut seek_pos = position;
        let mut accurate = accurate;
        self.state = match self.state {
            ControllerState::Seeking {
                seek_pos: _seek_pos,
                switch_to_play,
                keep_paused,
            } => ControllerState::Seeking {
                seek_pos: position,
                switch_to_play,
                keep_paused,
            },
            ControllerState::EOS | ControllerState::Ready => ControllerState::Seeking {
                seek_pos: position,
                switch_to_play: true,
                keep_paused: false,
            },
            ControllerState::Paused => {
                accurate = true;
                let seek_1st_step = self.audio_ctrl
                    .borrow()
                    .get_seek_back_1st_position(position);
                match seek_1st_step {
                    Some(seek_1st_step) => {
                        seek_pos = seek_1st_step;
                        ControllerState::TwoStepsSeek(position)
                    }
                    None => ControllerState::Seeking {
                        seek_pos: position,
                        switch_to_play: false,
                        keep_paused: true,
                    },
                }
            }
            ControllerState::TwoStepsSeek(target) => {
                must_sync_ctrl = true;
                seek_pos = target;
                ControllerState::Seeking {
                    seek_pos: position,
                    switch_to_play: false,
                    keep_paused: true,
                }
            }
            ControllerState::Playing => {
                must_sync_ctrl = true;
                ControllerState::Seeking {
                    seek_pos: position,
                    switch_to_play: false,
                    keep_paused: false,
                }
            }
            _ => return,
        };

        if must_sync_ctrl {
            self.info_ctrl.borrow_mut().seek(seek_pos, &self.state);
            self.audio_ctrl.borrow_mut().seek(seek_pos);
        }

        self.context.as_ref().unwrap().seek(seek_pos, accurate);
    }

    pub fn play_range(&mut self, start: u64, end: u64, pos_to_restore: u64) {
        if self.state == ControllerState::Paused {
            self.info_ctrl.borrow_mut().start_play_range();
            self.audio_ctrl.borrow_mut().start_play_range();

            self.state = ControllerState::PlayingRange(pos_to_restore);
            self.context.as_ref().unwrap().seek_range(start, end);
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.context.as_mut().unwrap().get_position()
    }

    pub fn refresh(&mut self) {
        self.audio_ctrl.borrow_mut().refresh();
    }

    pub fn refresh_info(&mut self, position: u64) {
        match self.state {
            ControllerState::Seeking { .. } => (),
            _ => self.info_ctrl.borrow_mut().tick(position, false),
        }
    }

    pub fn select_streams(&mut self, stream_ids: &[String]) {
        self.context.as_ref().unwrap().select_streams(stream_ids);
        // In Playing state, wait for the notification from the Context
        // Otherwise, update immediately
        if self.state != ControllerState::Playing {
            self.streams_selected();
        }
    }

    pub fn streams_selected(&mut self) {
        let context = self.context.take().unwrap();
        self.requires_async_dialog = context
            .info
            .lock()
            .unwrap()
            .streams
            .is_video_selected();

        {
            let info = context.info.lock().unwrap();
            self.audio_ctrl.borrow_mut().streams_changed(&info);
            self.info_ctrl.borrow().streams_changed(&info);
            self.perspective_ctrl.borrow().streams_changed(&info);
            self.split_ctrl.borrow_mut().streams_changed(&info);
            self.video_ctrl.streams_changed(&info);
        }
        self.set_context(context);
    }

    fn hold(&mut self) {
        self.switch_to_busy();
        self.audio_ctrl.borrow_mut().switch_to_not_playing();
        self.play_pause_btn.set_icon_name(PLAYBACK_ICON);

        if let Some(context) = self.context.as_mut() {
            context.pause().unwrap();
        };
    }

    pub fn request_context(&mut self, callback: Box<FnMut(PlaybackContext)>) {
        self.audio_ctrl.borrow_mut().switch_to_not_playing();
        self.play_pause_btn.set_icon_name(PLAYBACK_ICON);

        if let Some(context) = self.context.as_mut() {
            context.pause().unwrap();
        };

        let must_async = self.requires_async_dialog && self.state == ControllerState::Playing;
        self.take_context_cb = Some(callback);
        if must_async {
            self.state = ControllerState::PendingTakeContext;
        } else {
            self.have_context();
        }
    }

    fn have_context(&mut self) {
        if let Some(mut context) = self.context.take() {
            self.info_ctrl.borrow().export_chapters(&mut context);
            let mut callback = self.take_context_cb.take().unwrap();
            callback(context);
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

    fn handle_missing_plugins(&self) -> bool {
        if !self.missing_plugins.is_empty() {
            let mut missing_nb = 0;
            let mut missing_list = String::new();

            self.missing_plugins.iter().for_each(|missing_plugin| {
                if missing_nb > 0 {
                    missing_list += ", ";
                }

                missing_list += missing_plugin;
                missing_nb += 1;
            });

            let message = format!("{}",
                ngettext(
                    "Missing plugin: {}",
                    "Missing plugins: {}",
                    missing_nb
                ).replacen("{}", &missing_list, 1),
            );
            self.show_message(gtk::MessageType::Info, &message);
            error!("{}", message);

            true
        } else {
            false
        }
    }

    fn register_listener(&mut self, period: u32, ui_rx: Receiver<ContextMessage>) {
        if self.listener_src.is_some() {
            return;
        }

        let this_rc = Rc::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(period, move || {
            let mut keep_going = true;

            for message in ui_rx.try_iter() {
                match message {
                    AsyncDone => {
                        let mut this = this_rc.borrow_mut();
                        match this.state {
                            ControllerState::PendingSelectMedia => this.select_media(),
                            ControllerState::PendingTakeContext => this.have_context(),
                            ControllerState::Seeking {
                                seek_pos,
                                switch_to_play,
                                keep_paused,
                            } => {
                                if switch_to_play {
                                    this.context.as_mut().unwrap().play().unwrap();
                                    this.play_pause_btn.set_icon_name(PAUSE_ICON);
                                    this.state = ControllerState::Playing;
                                    this.audio_ctrl.borrow_mut().switch_to_playing();
                                } else if keep_paused {
                                    this.state = ControllerState::Paused;
                                    this.info_ctrl.borrow_mut().seek(seek_pos, &this.state);
                                    this.audio_ctrl.borrow_mut().seek(seek_pos);
                                } else {
                                    this.state = ControllerState::Playing;
                                }
                            }
                            _ => (),
                        }
                    }
                    InitDone => {
                        let mut this = this_rc.borrow_mut();
                        let mut context = this.context.take().unwrap();

                        this.requires_async_dialog = context
                            .info
                            .lock()
                            .unwrap()
                            .streams
                            .is_video_selected();

                        this.header_bar
                            .set_subtitle(Some(context.file_name.as_str()));

                        this.audio_ctrl.borrow_mut().new_media(&context);
                        this.export_ctrl.borrow_mut().new_media();
                        this.info_ctrl.borrow_mut().new_media(&context);
                        this.perspective_ctrl.borrow().new_media(&context);
                        this.split_ctrl.borrow_mut().new_media(&context);
                        this.streams_ctrl.borrow_mut().new_media(&context);
                        this.video_ctrl.new_media(&context);

                        this.set_context(context);

                        this.handle_missing_plugins();
                        this.state = ControllerState::Ready;
                    }
                    MissingPlugin(plugin) => {
                        error!("{}", gettext("Missing plugin: {}").replacen("{}", &plugin, 1));
                        this_rc.borrow_mut().missing_plugins.insert(plugin);
                    }
                    ReadyForRefresh => {
                        let mut this = this_rc.borrow_mut();
                        match this.state {
                            ControllerState::Paused => this.refresh(),
                            ControllerState::TwoStepsSeek(target) => this.seek(target, true),
                            _ => (),
                        }
                    }
                    StreamsSelected => this_rc.borrow_mut().streams_selected(),
                    Eos => {
                        let mut this = this_rc.borrow_mut();
                        match this.state {
                            ControllerState::PlayingRange(pos_to_restore) => {
                                // end of range => pause and seek back to pos_to_restore
                                this.context.as_ref().unwrap().pause().unwrap();
                                this.state = ControllerState::Paused;
                                this.audio_ctrl.borrow_mut().stop_play_range();
                                this.seek(pos_to_restore, true); // accurate
                            }
                            _ => {
                                this.play_pause_btn.set_icon_name(PLAYBACK_ICON);
                                this.state = ControllerState::EOS;

                                // The tick callback will be register again in case of a seek
                                this.audio_ctrl.borrow_mut().switch_to_not_playing();
                            }
                        }
                    }
                    FailedToOpenMedia(error) => {
                        let mut this = this_rc.borrow_mut();
                        this.context = None;
                        this.state = ControllerState::Stopped;
                        this.switch_to_default();

                        this.keep_going = false;
                        keep_going = false;

                        if !this.missing_plugins.is_empty() {
                            this.handle_missing_plugins();
                        } else {
                            let error = gettext("Error opening file. {}").replacen("{}", &error, 1);
                            this.show_message(gtk::MessageType::Error, &error);
                            error!("{}", error);
                        }
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
                this.audio_ctrl.borrow_mut().switch_to_not_playing();
            }

            glib::Continue(keep_going)
        }));
    }

    pub fn set_cursor_waiting(&self) {
        let gdk_window = self.window.get_window().unwrap();
        gdk_window.set_cursor(&Cursor::new_for_display(
            &gdk_window.get_display(),
            CursorType::Watch,
        ));
    }

    pub fn reset_cursor(&self) {
        self.window.get_window().unwrap().set_cursor(None);
    }

    fn switch_to_busy(&mut self) {
        self.window.set_sensitive(false);
        self.set_cursor_waiting();
    }

    fn switch_to_default(&mut self) {
        self.reset_cursor();
        self.window.set_sensitive(true);
    }

    fn select_media(&mut self) {
        self.switch_to_busy();
        self.info_bar.hide();

        let file_dlg = gtk::FileChooserDialog::new(
            Some(&gettext("Open a media file")),
            Some(&self.window),
            gtk::FileChooserAction::Open,
        );
        // Note: couldn't find equivalents for STOCK_OK
        file_dlg.add_button(&gettext("Open"), gtk::ResponseType::Ok.into());

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

    pub fn open_media(&mut self, filepath: PathBuf) {
        self.remove_listener();

        self.info_ctrl.borrow_mut().cleanup();
        self.audio_ctrl.borrow_mut().cleanup();
        self.video_ctrl.cleanup();
        self.export_ctrl.borrow_mut().cleanup();
        self.split_ctrl.borrow_mut().cleanup();
        self.streams_ctrl.borrow_mut().cleanup();
        self.perspective_ctrl.borrow().cleanup();
        self.header_bar.set_subtitle("");

        let (ctx_tx, ui_rx) = channel();

        self.state = ControllerState::Stopped;
        self.missing_plugins.clear();
        self.keep_going = true;
        self.register_listener(LISTENER_PERIOD, ui_rx);

        let dbl_buffer_mtx = Arc::clone(&self.audio_ctrl.borrow().get_dbl_buffer_mtx());
        match PlaybackContext::new(filepath, dbl_buffer_mtx, ctx_tx) {
            Ok(context) => {
                self.context = Some(context);
            }
            Err(error) => {
                self.switch_to_default();
                let error = gettext("Error opening file. {}").replace("{}", &error);
                self.show_message(gtk::MessageType::Error, &error);
                error!("{}", error);
            }
        };
    }
}
