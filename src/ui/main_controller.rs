use std::cell::RefCell;
use std::rc::Rc;

use std::collections::HashSet;

use std::path::PathBuf;

use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver};

use gettextrs::{gettext, ngettext};
use gio;
use gio::prelude::*;
use gio::MenuExt;

use glib;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use gdk::{Cursor, CursorType, WindowExt};

use application::{APP_ID, APP_PATH, CONFIG};
use media::ContextMessage::*;
use media::{ContextMessage, PlaybackContext};

use super::{AudioController, ChaptersBoundaries, ExportController, InfoController,
            PerspectiveController, SplitController, StreamsController, VideoController};

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
    playback_paned: gtk::Paned,
    play_pause_btn: gtk::ToolButton,
    info_bar_revealer: gtk::Revealer,
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

    this_opt: Option<Rc<RefCell<MainController>>>,
    keep_going: bool,
    listener_src: Option<glib::SourceId>,
}

impl MainController {
    pub fn new(
        gtk_app: &gtk::Application,
        is_gst_ok: bool,
        disable_gl: bool,
    ) -> Rc<RefCell<Self>> {
        let builder = gtk::Builder::new_from_resource(&format!("{}/{}", *APP_PATH, "media-toc.ui"));
        let window: gtk::ApplicationWindow = builder.get_object("application-window").unwrap();
        window.set_application(gtk_app);

        let chapters_boundaries = Rc::new(RefCell::new(ChaptersBoundaries::new()));

        let this = Rc::new(RefCell::new(MainController {
            window,
            header_bar: builder.get_object("header-bar").unwrap(),
            open_btn: builder.get_object("open-btn").unwrap(),
            playback_paned: builder.get_object("playback-paned").unwrap(),
            play_pause_btn: builder.get_object("play_pause-toolbutton").unwrap(),
            info_bar_revealer: builder.get_object("info_bar-revealer").unwrap(),
            info_bar: builder.get_object("info_bar").unwrap(),
            info_bar_lbl: builder.get_object("info_bar-lbl").unwrap(),

            perspective_ctrl: PerspectiveController::new(&builder),
            video_ctrl: VideoController::new(&builder, disable_gl),
            info_ctrl: InfoController::new(&builder, Rc::clone(&chapters_boundaries)),
            audio_ctrl: AudioController::new(&builder, chapters_boundaries),
            export_ctrl: ExportController::new(&builder),
            split_ctrl: SplitController::new(&builder),
            streams_ctrl: StreamsController::new(&builder),

            context: None,
            take_context_cb: None,
            missing_plugins: HashSet::<String>::new(),
            state: ControllerState::Stopped,

            this_opt: None,
            keep_going: true,
            listener_src: None,
        }));

        {
            let mut this_mut = this.borrow_mut();

            let this_rc = Rc::clone(&this);
            this_mut.this_opt = Some(this_rc);

            let app_menu = gio::Menu::new();
            gtk_app.set_app_menu(&app_menu);

            let app_section = gio::Menu::new();
            app_menu.append_section(None, &app_section);

            // Register About action
            let about = gio::SimpleAction::new("about", None);
            gtk_app.add_action(&about);
            let this_rc = Rc::clone(&this);
            about.connect_activate(move |_, _| this_rc.borrow().about());
            gtk_app.set_accels_for_action("app.about", &["<Ctrl>A"]);
            app_section.append(&gettext("About")[..], "app.about");

            // Register Quit action
            let quit = gio::SimpleAction::new("quit", None);
            gtk_app.add_action(&quit);
            let this_rc = Rc::clone(&this);
            quit.connect_activate(move |_, _| this_rc.borrow_mut().quit());
            gtk_app.set_accels_for_action("app.quit", &["<Ctrl>Q"]);
            app_section.append(&gettext("Quit")[..], "app.quit");

            let this_rc = Rc::clone(&this);
            this_mut.window.connect_delete_event(move |_, _| {
                this_rc.borrow_mut().quit();
                Inhibit(false)
            });

            // Prepare controllers
            if is_gst_ok {
                {
                    let config = CONFIG.read().unwrap();
                    if config.ui.width > 0 && config.ui.height > 0 {
                        this_mut.window.resize(config.ui.width, config.ui.height);
                        this_mut.playback_paned.set_position(config.ui.paned_pos);
                    }
                }

                this_mut.video_ctrl.register_callbacks(&this);
                PerspectiveController::register_callbacks(
                    &this_mut.perspective_ctrl,
                    gtk_app,
                    &this,
                );
                InfoController::register_callbacks(&this_mut.info_ctrl, gtk_app, &this);
                AudioController::register_callbacks(&this_mut.audio_ctrl, gtk_app, &this);
                ExportController::register_callbacks(&this_mut.export_ctrl, &this);
                SplitController::register_callbacks(&this_mut.split_ctrl, &this);
                StreamsController::register_callbacks(&this_mut.streams_ctrl, &this);

                let _ = PlaybackContext::check_requirements().map_err(|err| {
                    error!("{}", err);
                    let this_rc = Rc::clone(&this);
                    gtk::idle_add(move || {
                        this_rc
                            .borrow()
                            .show_message(gtk::MessageType::Warning, &err);
                        glib::Continue(false)
                    });
                });

                let main_section = gio::Menu::new();
                app_menu.insert_section(0, None, &main_section);

                // Register Open action
                let open = gio::SimpleAction::new("open", None);
                gtk_app.add_action(&open);
                let this_rc = Rc::clone(&this);
                open.connect_activate(move |_, _| {
                    let mut this = this_rc.borrow_mut();

                    if this.state == ControllerState::Playing || this.state == ControllerState::EOS
                    {
                        this.hold();
                        this.state = ControllerState::PendingSelectMedia;
                    } else {
                        this.select_media();
                    }
                });
                gtk_app.set_accels_for_action("app.open", &["<Ctrl>O"]);
                main_section.append(&gettext("Open media file")[..], "app.open");

                this_mut.open_btn.set_sensitive(true);

                // Register Play/Pause action
                let play_pause = gio::SimpleAction::new("play_pause", None);
                gtk_app.add_action(&play_pause);
                let this_rc = Rc::clone(&this);
                play_pause.connect_activate(move |_, _| {
                    this_rc.borrow_mut().play_pause();
                });
                gtk_app.set_accels_for_action("app.play_pause", &["space", "P"]);

                this_mut.play_pause_btn.set_sensitive(true);

                // Register Close info bar action
                let close_info_bar = gio::SimpleAction::new("close_info_bar", None);
                gtk_app.add_action(&close_info_bar);
                let revealer = this_mut.info_bar_revealer.clone();
                close_info_bar.connect_activate(move |_, _| revealer.set_reveal_child(false));
                gtk_app.set_accels_for_action("app.close_info_bar", &["Escape"]);

                let revealer = this_mut.info_bar_revealer.clone();
                this_mut
                    .info_bar
                    .connect_response(move |_, _| revealer.set_reveal_child(false));
            } else {
                // GStreamer initialization failed

                // Register Close info bar action
                let close_info_bar = gio::SimpleAction::new("close_info_bar", None);
                gtk_app.add_action(&close_info_bar);
                let this_rc = Rc::clone(&this);
                close_info_bar.connect_activate(move |_, _| this_rc.borrow_mut().quit());
                gtk_app.set_accels_for_action("app.close_info_bar", &["Escape"]);

                let this_rc = Rc::clone(&this);
                this_mut.info_bar.connect_response(move |_, _| this_rc.borrow_mut().quit());

                let msg = gettext("Failed to initialize GStreamer, the application can't be used.");
                this_mut.show_message(gtk::MessageType::Error, &msg);
                error!("{}", msg);
            }
        }

        this
    }

    pub fn show_all(&self) {
        self.window.show();
        self.window.activate();
    }

    fn about(&self) {
        let dialog = gtk::AboutDialog::new();
        dialog.set_modal(true);
        dialog.set_transient_for(&self.window);

        dialog.set_program_name(env!("CARGO_PKG_NAME"));
        dialog.set_logo_icon_name(&APP_ID[..]);
        dialog.set_comments(&gettext(
            "Build a table of contents from a media file\nor split a media file into chapters"
        )[..]);
        dialog.set_copyright(&gettext("© 2017–2018 François Laignel")[..]);
        dialog.set_license_type(gtk::License::MitX11);
        dialog.set_version(env!("CARGO_PKG_VERSION"));
        dialog.set_website(env!("CARGO_PKG_HOMEPAGE"));
        dialog.set_website_label(&gettext("Learn more about media-toc")[..]);

        dialog.show();
    }

    fn quit(&mut self) {
        if let Some(context) = self.context.take() {
            context.stop();
        }
        self.remove_listener();

        {
            let size = self.window.get_size();
            let paned_pos = self.playback_paned.get_position();
            let mut config = CONFIG.write().unwrap();
            config.ui.width = size.0;
            config.ui.height = size.1;
            config.ui.paned_pos = paned_pos;
            config.save();
        }

        self.window.destroy();
    }

    pub fn show_message(&self, type_: gtk::MessageType, message: &str) {
        self.info_bar.set_message_type(type_);
        self.info_bar_lbl.set_label(message);
        self.info_bar_revealer.set_reveal_child(true);
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
        {
            let info = context.info.read().unwrap();
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

        self.take_context_cb = Some(callback);
        if self.state == ControllerState::Playing || self.state == ControllerState::EOS {
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

    fn check_missing_plugins(&self) -> Option<String> {
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
            let message = ngettext("Missing plugin: {}", "Missing plugins: {}", missing_nb)
                .replacen("{}", &missing_list, 1);

            Some(message)
        } else {
            None
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
                        if let ControllerState::Seeking {
                            seek_pos,
                            switch_to_play,
                            keep_paused,
                        } = this.state
                        {
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
                    }
                    InitDone => {
                        let mut this = this_rc.borrow_mut();
                        let mut context = this.context.take().unwrap();

                        this.header_bar
                            .set_subtitle(Some(context.info.read().unwrap().file_name.as_str()));

                        this.audio_ctrl.borrow_mut().new_media(&context);
                        this.export_ctrl.borrow_mut().new_media();
                        this.info_ctrl.borrow_mut().new_media(&context);
                        this.perspective_ctrl.borrow().new_media(&context);
                        this.split_ctrl.borrow_mut().new_media(&context);
                        this.streams_ctrl.borrow_mut().new_media(&context);
                        this.video_ctrl.new_media(&context);

                        this.set_context(context);

                        if let Some(message) = this.check_missing_plugins() {
                            this.show_message(gtk::MessageType::Info, &message);
                            error!("{}", message);
                        }
                        this.state = ControllerState::Ready;
                    }
                    MissingPlugin(plugin) => {
                        error!(
                            "{}",
                            gettext("Missing plugin: {}").replacen("{}", &plugin, 1)
                        );
                        this_rc.borrow_mut().missing_plugins.insert(plugin);
                    }
                    ReadyForRefresh => {
                        let mut this = this_rc.borrow_mut();
                        match this.state {
                            ControllerState::Paused | ControllerState::Ready => this.refresh(),
                            ControllerState::TwoStepsSeek(target) => this.seek(target, true),
                            ControllerState::PendingSelectMedia => this.select_media(),
                            ControllerState::PendingTakeContext => this.have_context(),
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

                        let mut error = gettext("Error opening file.\n\n{}").replacen("{}", &error, 1);
                        if let Some(message) = this.check_missing_plugins() {
                            error += "\n\n";
                            error += &message;
                        }
                        this.show_message(gtk::MessageType::Error, &error);
                        error!("{}", error);
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
        self.info_bar_revealer.set_reveal_child(false);
        self.switch_to_busy();

        let file_dlg = gtk::FileChooserDialog::with_buttons(
            Some(&gettext("Open a media file")),
            Some(&self.window),
            gtk::FileChooserAction::Open,
            &[
                (&gettext("Cancel"), gtk::ResponseType::Cancel),
                (&gettext("Open"), gtk::ResponseType::Accept),
            ],
        );
        if let Some(ref last_path) = CONFIG.read().unwrap().media.last_path {
            file_dlg.set_current_folder(last_path);
        }

        if file_dlg.run() == gtk::ResponseType::Accept.into() {
            if let Some(ref context) = self.context.take() {
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

        let dbl_buffer_mtx = Arc::clone(&self.audio_ctrl.borrow().dbl_buffer_mtx);
        match PlaybackContext::new(
            &filepath,
            &dbl_buffer_mtx,
            self.video_ctrl.get_video_sink(),
            ctx_tx,
        ) {
            Ok(context) => {
                CONFIG
                    .write()
                    .unwrap()
                    .media.last_path = filepath.parent().map(|path| path.to_owned());
                self.context = Some(context);
            }
            Err(error) => {
                self.switch_to_default();
                let error = gettext("Error opening file.\n\n{}").replace("{}", &error);
                self.show_message(gtk::MessageType::Error, &error);
                error!("{}", error);
            }
        };
    }
}
