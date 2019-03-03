use gdk::{Cursor, CursorType, WindowExt};
use gettextrs::{gettext, ngettext};
use gio;
use gio::prelude::*;

use glib;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use log::{error, info};

use std::{
    cell::RefCell,
    collections::HashSet,
    path::Path,
    rc::{Rc, Weak},
    sync::Arc,
};

use crate::{
    application::{APP_ID, APP_PATH, CONFIG},
    media::{MediaEvent, PlaybackPipeline},
};

use super::{
    AudioController, ChaptersBoundaries, ExportController, InfoController, PerspectiveController,
    PositionStatus, SplitController, StreamsController, UIController, VideoController,
};

const PAUSE_ICON: &str = "media-playback-pause-symbolic";
const PLAYBACK_ICON: &str = "media-playback-start-symbolic";

#[derive(Clone, Copy, PartialEq)]
pub enum PostSeekAction {
    KeepPaused,
    SwitchToPlay,
    KeepPlaying,
}

#[derive(PartialEq)]
pub enum ControllerState {
    EOS,
    Paused,
    PendingTakePipeline,
    PendingSelectMedia,
    Playing,
    PlayingRange(u64),
    Ready,
    Seeking {
        seek_pos: u64,
        post_seek_action: PostSeekAction,
    },
    Stopped,
    TwoStepsSeek(u64),
}

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

    pub pipeline: Option<PlaybackPipeline>,
    take_pipeline_cb: Option<Box<dyn FnMut(PlaybackPipeline)>>,
    missing_plugins: HashSet<String>,
    state: ControllerState,

    this_opt: Option<Weak<RefCell<MainController>>>,
    media_event_handler_src: Option<glib::SourceId>,
}

impl MainController {
    pub fn new_rc(
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

            perspective_ctrl: PerspectiveController::new_rc(&builder),
            video_ctrl: VideoController::new(&builder, disable_gl),
            info_ctrl: InfoController::new_rc(&builder, Rc::clone(&chapters_boundaries)),
            audio_ctrl: AudioController::new_rc(&builder, chapters_boundaries),
            export_ctrl: ExportController::new_rc(&builder),
            split_ctrl: SplitController::new_rc(&builder),
            streams_ctrl: StreamsController::new_rc(&builder),

            pipeline: None,
            take_pipeline_cb: None,
            missing_plugins: HashSet::<String>::new(),
            state: ControllerState::Stopped,

            this_opt: None,
            media_event_handler_src: None,
        }));

        {
            let mut this_mut = this.borrow_mut();

            this_mut.this_opt = Some(Rc::downgrade(&this));

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

                let _ = PlaybackPipeline::check_requirements().map_err(|err| {
                    let this_rc = Rc::clone(&this);
                    gtk::idle_add(move || {
                        this_rc.borrow().show_error(&err);
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
                gtk_app.set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);

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
                this_mut
                    .info_bar
                    .connect_response(move |_, _| this_rc.borrow_mut().quit());

                let msg = gettext("Failed to initialize GStreamer, the application can't be used.");
                this_mut.show_error(msg);
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
        dialog.set_comments(
            &gettext(
                "Build a table of contents from a media file\nor split a media file into chapters",
            )[..],
        );
        dialog.set_copyright(&gettext("© 2017–2019 François Laignel")[..]);
        dialog.set_translator_credits(&gettext("translator-credits")[..]);
        dialog.set_license_type(gtk::License::MitX11);
        dialog.set_version(env!("CARGO_PKG_VERSION"));
        dialog.set_website(env!("CARGO_PKG_HOMEPAGE"));
        dialog.set_website_label(&gettext("Learn more about media-toc")[..]);

        dialog.connect_response(|dialog, _| dialog.close());
        dialog.show();
    }

    fn quit(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            pipeline.stop();
        }
        self.remove_media_event_handler();

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

    pub fn show_message<Msg: AsRef<str>>(&self, type_: gtk::MessageType, message: Msg) {
        self.info_bar.set_message_type(type_);
        self.info_bar_lbl.set_label(message.as_ref());
        self.info_bar_revealer.set_reveal_child(true);
    }

    pub fn show_error<Msg: AsRef<str>>(&self, message: Msg) {
        error!("{}", message.as_ref());
        self.show_message(gtk::MessageType::Error, message);
    }

    pub fn show_info<Msg: AsRef<str>>(&self, message: Msg) {
        info!("{}", message.as_ref());
        self.show_message(gtk::MessageType::Info, message);
    }

    pub fn play_pause(&mut self) {
        let mut pipeline = match self.pipeline.take() {
            Some(pipeline) => pipeline,
            None => {
                self.select_media();
                return;
            }
        };

        if self.state != ControllerState::EOS {
            match pipeline.get_state() {
                gst::State::Paused => {
                    self.play_pause_btn.set_icon_name(PAUSE_ICON);
                    self.state = ControllerState::Playing;
                    self.audio_ctrl.borrow_mut().switch_to_playing();
                    pipeline.play().unwrap();
                    self.pipeline = Some(pipeline);
                }
                gst::State::Playing => {
                    pipeline.pause().unwrap();
                    self.play_pause_btn.set_icon_name(PLAYBACK_ICON);
                    self.state = ControllerState::Paused;
                    self.audio_ctrl.borrow_mut().switch_to_not_playing();
                    self.pipeline = Some(pipeline);
                }
                _ => {
                    self.pipeline = Some(pipeline);
                    self.select_media();
                }
            };
        } else {
            // Restart the stream from the begining
            self.pipeline = Some(pipeline);
            self.seek(0, gst::SeekFlags::ACCURATE);
        }
    }

    pub fn move_chapter_boundary(&mut self, boundary: u64, to_position: u64) -> PositionStatus {
        self.info_ctrl
            .borrow_mut()
            .move_chapter_boundary(boundary, to_position)
    }

    pub fn seek(&mut self, position: u64, flags: gst::SeekFlags) {
        let mut must_sync_ctrl = false;
        let mut seek_pos = position;
        let mut flags = flags;
        match self.state {
            ControllerState::Seeking { .. } => (),
            ControllerState::EOS | ControllerState::Ready => {
                self.state = ControllerState::Seeking {
                    seek_pos: position,
                    post_seek_action: PostSeekAction::SwitchToPlay,
                };
            }
            ControllerState::Paused => {
                flags = gst::SeekFlags::ACCURATE;
                let seek_1st_step = self
                    .audio_ctrl
                    .borrow()
                    .get_seek_back_1st_position(position);
                self.state = match seek_1st_step {
                    Some(seek_1st_step) => {
                        seek_pos = seek_1st_step;
                        ControllerState::TwoStepsSeek(position)
                    }
                    None => ControllerState::Seeking {
                        seek_pos: position,
                        post_seek_action: PostSeekAction::KeepPaused,
                    },
                };
            }
            ControllerState::TwoStepsSeek(target) => {
                must_sync_ctrl = true;
                seek_pos = target;
                self.state = ControllerState::Seeking {
                    seek_pos: position,
                    post_seek_action: PostSeekAction::KeepPaused,
                };
            }
            ControllerState::Playing => {
                must_sync_ctrl = true;
                self.state = ControllerState::Seeking {
                    seek_pos: position,
                    post_seek_action: PostSeekAction::KeepPlaying,
                };
            }
            _ => return,
        };

        if must_sync_ctrl {
            self.info_ctrl.borrow_mut().seek(seek_pos, &self.state);
            self.audio_ctrl.borrow_mut().seek(seek_pos);
        }

        self.pipeline.as_ref().unwrap().seek(seek_pos, flags);
    }

    pub fn play_range(&mut self, start: u64, end: u64, pos_to_restore: u64) {
        if self.state == ControllerState::Paused {
            self.info_ctrl.borrow_mut().start_play_range();
            self.audio_ctrl.borrow_mut().start_play_range();

            self.state = ControllerState::PlayingRange(pos_to_restore);
            self.pipeline.as_ref().unwrap().seek_range(start, end);
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.pipeline.as_mut().unwrap().get_position()
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

    pub fn select_streams(&mut self, stream_ids: &[Arc<str>]) {
        self.pipeline.as_ref().unwrap().select_streams(stream_ids);
        // In Playing state, wait for the notification from the pipeline
        // Otherwise, update immediately
        if self.state != ControllerState::Playing {
            self.streams_selected();
        }
    }

    pub fn streams_selected(&mut self) {
        let pipeline = self.pipeline.take().unwrap();
        {
            let info = pipeline.info.read().unwrap();
            self.audio_ctrl.borrow_mut().streams_changed(&info);
            self.info_ctrl.borrow_mut().streams_changed(&info);
            self.perspective_ctrl.borrow_mut().streams_changed(&info);
            self.split_ctrl.borrow_mut().streams_changed(&info);
            self.video_ctrl.streams_changed(&info);
        }
        self.set_pipeline(pipeline);
    }

    fn hold(&mut self) {
        self.switch_to_busy();
        self.audio_ctrl.borrow_mut().switch_to_not_playing();
        self.play_pause_btn.set_icon_name(PLAYBACK_ICON);

        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.pause().unwrap();
        };
    }

    pub fn request_pipeline(&mut self, callback: Box<dyn FnMut(PlaybackPipeline)>) {
        self.audio_ctrl.borrow_mut().switch_to_not_playing();
        self.play_pause_btn.set_icon_name(PLAYBACK_ICON);

        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.pause().unwrap();
        };

        self.take_pipeline_cb = Some(callback);
        if self.state == ControllerState::Playing || self.state == ControllerState::EOS {
            self.state = ControllerState::PendingTakePipeline;
        } else {
            self.have_pipeline();
        }
    }

    fn have_pipeline(&mut self) {
        if let Some(mut pipeline) = self.pipeline.take() {
            self.info_ctrl.borrow().export_chapters(&mut pipeline);
            let mut callback = self.take_pipeline_cb.take().unwrap();
            callback(pipeline);
            self.state = ControllerState::Paused;
        }
    }

    pub fn set_pipeline(&mut self, pipeline: PlaybackPipeline) {
        self.pipeline = Some(pipeline);
        self.state = ControllerState::Paused;
        self.switch_to_default();
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

    fn register_media_event_handler(&mut self, receiver: glib::Receiver<MediaEvent>) {
        let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

        self.media_event_handler_src = Some(receiver.attach(None, move |event| {
            let this_rc = this_weak
                .upgrade()
                .expect("Lost `MainController` in `MediaEvent` handler");
            let mut this = this_rc.borrow_mut();
            this.handle_media_event(event)
        }));
    }

    fn remove_media_event_handler(&mut self) {
        if let Some(source_id) = self.media_event_handler_src.take() {
            glib::source_remove(source_id);
        }
    }

    fn handle_media_event(&mut self, event: MediaEvent) -> glib::Continue {
        let mut keep_going = true;

        match event {
            MediaEvent::AsyncDone => {
                if let ControllerState::Seeking {
                    seek_pos,
                    post_seek_action,
                } = self.state
                {
                    match post_seek_action {
                        PostSeekAction::SwitchToPlay => {
                            self.pipeline.as_mut().unwrap().play().unwrap();
                            self.play_pause_btn.set_icon_name(PAUSE_ICON);
                            self.state = ControllerState::Playing;
                            self.audio_ctrl.borrow_mut().switch_to_playing();
                        }
                        PostSeekAction::KeepPaused => {
                            self.state = ControllerState::Paused;
                            self.info_ctrl.borrow_mut().seek(seek_pos, &self.state);
                            self.audio_ctrl.borrow_mut().seek(seek_pos);
                        }
                        PostSeekAction::KeepPlaying => {
                            self.state = ControllerState::Playing;
                        }
                    }
                }
            }
            MediaEvent::InitDone => {
                let pipeline = self.pipeline.take().unwrap();

                self.header_bar
                    .set_subtitle(Some(pipeline.info.read().unwrap().file_name.as_str()));

                self.audio_ctrl.borrow_mut().new_media(&pipeline);
                self.export_ctrl.borrow_mut().new_media(&pipeline);
                self.info_ctrl.borrow_mut().new_media(&pipeline);
                self.perspective_ctrl.borrow_mut().new_media(&pipeline);
                self.split_ctrl.borrow_mut().new_media(&pipeline);
                self.streams_ctrl.borrow_mut().new_media(&pipeline);
                self.video_ctrl.new_media(&pipeline);

                self.set_pipeline(pipeline);

                if let Some(message) = self.check_missing_plugins() {
                    self.show_error(message);
                }
                self.state = ControllerState::Ready;
            }
            MediaEvent::MissingPlugin(plugin) => {
                error!(
                    "{}",
                    gettext("Missing plugin: {}").replacen("{}", &plugin, 1)
                );
                self.missing_plugins.insert(plugin);
            }
            MediaEvent::ReadyForRefresh => match self.state {
                ControllerState::Paused | ControllerState::Ready => self.refresh(),
                ControllerState::TwoStepsSeek(target) => {
                    self.seek(target, gst::SeekFlags::ACCURATE)
                }
                ControllerState::PendingSelectMedia => self.select_media(),
                ControllerState::PendingTakePipeline => self.have_pipeline(),
                _ => (),
            },
            MediaEvent::StreamsSelected => self.streams_selected(),
            MediaEvent::Eos => {
                match self.state {
                    ControllerState::PlayingRange(pos_to_restore) => {
                        // end of range => pause and seek back to pos_to_restore
                        self.pipeline.as_ref().unwrap().pause().unwrap();
                        self.state = ControllerState::Paused;
                        self.audio_ctrl.borrow_mut().stop_play_range();
                        self.seek(pos_to_restore, gst::SeekFlags::ACCURATE);
                    }
                    _ => {
                        self.play_pause_btn.set_icon_name(PLAYBACK_ICON);
                        self.state = ControllerState::EOS;

                        // The tick callback will be register again in case of a seek
                        self.audio_ctrl.borrow_mut().switch_to_not_playing();
                    }
                }
            }
            MediaEvent::FailedToOpenMedia(error) => {
                self.pipeline = None;
                self.state = ControllerState::Stopped;
                self.switch_to_default();

                keep_going = false;

                let mut error = gettext("Error opening file.\n\n{}").replacen("{}", &error, 1);
                if let Some(message) = self.check_missing_plugins() {
                    error += "\n\n";
                    error += &message;
                }
                self.show_error(error);
            }
            _ => (),
        }

        if !keep_going {
            self.remove_media_event_handler();
            self.audio_ctrl.borrow_mut().switch_to_not_playing();
        }

        glib::Continue(keep_going)
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
            Some(gettext("Open a media file").as_str()),
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
            if let Some(ref pipeline) = self.pipeline.take() {
                pipeline.stop();
            }
            self.open_media(&file_dlg.get_filename().unwrap());
        } else {
            if self.pipeline.is_some() {
                self.state = ControllerState::Paused;
            }
            self.switch_to_default();
        }

        file_dlg.close();
    }

    pub fn open_media(&mut self, filepath: &Path) {
        self.remove_media_event_handler();

        self.info_ctrl.borrow_mut().cleanup();
        self.audio_ctrl.borrow_mut().cleanup();
        self.video_ctrl.cleanup();
        self.export_ctrl.borrow_mut().cleanup();
        self.split_ctrl.borrow_mut().cleanup();
        self.streams_ctrl.borrow_mut().cleanup();
        self.perspective_ctrl.borrow_mut().cleanup();
        self.header_bar.set_subtitle("");

        let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);

        self.state = ControllerState::Stopped;
        self.missing_plugins.clear();
        self.register_media_event_handler(receiver);

        let dbl_buffer_mtx = Arc::clone(&self.audio_ctrl.borrow().dbl_buffer_mtx);
        match PlaybackPipeline::try_new(
            filepath,
            &dbl_buffer_mtx,
            &self.video_ctrl.get_video_sink(),
            sender,
        ) {
            Ok(pipeline) => {
                CONFIG.write().unwrap().media.last_path =
                    filepath.parent().map(|path| path.to_owned());
                self.pipeline = Some(pipeline);
            }
            Err(error) => {
                self.switch_to_default();
                let error = gettext("Error opening file.\n\n{}").replace("{}", &error);
                self.show_error(error);
            }
        };
    }
}
