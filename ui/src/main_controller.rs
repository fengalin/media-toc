use futures::channel::mpsc as async_mpsc;
use futures::future::{abortable, AbortHandle, LocalBoxFuture};
use futures::prelude::*;

use gettextrs::{gettext, ngettext};

use gstreamer as gst;
use gtk::prelude::*;

use log::{debug, error};

use std::{borrow::ToOwned, cell::RefCell, collections::HashSet, path::PathBuf, rc::Rc, sync::Arc};

use application::{CommandLineArguments, APP_ID, APP_PATH, CONFIG};
use media::{MediaEvent, PlaybackState, Timestamp};

use crate::spawn;

use super::{
    AudioController, ChaptersBoundaries, ExportController, InfoController, MainDispatcher,
    MediaEventReceiver, PerspectiveController, PlaybackPipeline, PositionStatus, SplitController,
    StreamsController, UIController, UIEventHandler, UIEventSender, VideoController,
};

const PAUSE_ICON: &str = "media-playback-pause-symbolic";
const PLAYBACK_ICON: &str = "media-playback-start-symbolic";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControllerState {
    EOS,
    Paused,
    PendingPaused,
    PendingSelectMedia,
    PendingSelectMediaDecision,
    Playing,
    PlayingRange(Timestamp),
    Seeking,
    Stopped,
    TwoStepsSeek(Timestamp),
}

pub struct MainController {
    pub(super) window: gtk::ApplicationWindow,

    header_bar: gtk::HeaderBar,
    pub(super) open_btn: gtk::Button,
    pub(super) display_page: gtk::Box,
    playback_paned: gtk::Paned,
    pub(super) play_pause_btn: gtk::ToolButton,
    file_dlg: gtk::FileChooserNative,

    ui_event: UIEventSender,

    pub(super) perspective_ctrl: PerspectiveController,
    pub(super) video_ctrl: VideoController,
    pub(super) info_ctrl: InfoController,
    pub(super) audio_ctrl: AudioController,
    pub(super) export_ctrl: ExportController,
    pub(super) split_ctrl: SplitController,
    pub(super) streams_ctrl: StreamsController,

    pub(super) pipeline: Option<PlaybackPipeline>,
    missing_plugins: HashSet<String>,
    pub(super) state: ControllerState,

    pub(super) new_media_event_handler:
        Option<Box<dyn Fn(MediaEventReceiver) -> LocalBoxFuture<'static, ()>>>,
    media_event_abort_handle: Option<AbortHandle>,
    callback_when_paused: Option<Box<dyn Fn(&mut MainController)>>,
}

impl MainController {
    pub fn setup(app: &gtk::Application, args: &CommandLineArguments) {
        let builder = gtk::Builder::from_resource(&format!("{}/{}", *APP_PATH, "media-toc.ui"));

        let window: gtk::ApplicationWindow = builder.get_object("application-window").unwrap();
        window.set_application(Some(app));

        let (mut ui_event_handler, ui_event) = UIEventHandler::new_pair(&app, &builder);
        let chapters_boundaries = Rc::new(RefCell::new(ChaptersBoundaries::new()));

        let file_dlg = gtk::FileChooserNativeBuilder::new()
            .title(&gettext("Open a media file"))
            .transient_for(&window)
            .modal(true)
            .accept_label(&gettext("Open"))
            .cancel_label(&gettext("Cancel"))
            .build();

        let ui_event_clone = ui_event.clone();
        file_dlg.connect_response(move |file_dlg, response| {
            file_dlg.hide();
            match (response, file_dlg.get_filename()) {
                (gtk::ResponseType::Accept, Some(path)) => ui_event_clone.open_media(path),
                _ => ui_event_clone.cancel_select_media(),
            }
        });

        let gst_init_res = gst::init();

        let main_ctrl_rc = Rc::new(RefCell::new(MainController {
            window,
            header_bar: builder.get_object("header-bar").unwrap(),
            open_btn: builder.get_object("open-btn").unwrap(),
            display_page: builder.get_object("display-box").unwrap(),
            playback_paned: builder.get_object("playback-paned").unwrap(),
            play_pause_btn: builder.get_object("play_pause-toolbutton").unwrap(),
            file_dlg,

            ui_event: ui_event.clone(),

            perspective_ctrl: PerspectiveController::new(&builder),
            video_ctrl: VideoController::new(&builder, args),
            info_ctrl: InfoController::new(
                &builder,
                ui_event.clone(),
                Rc::clone(&chapters_boundaries),
            ),
            audio_ctrl: AudioController::new(&builder, ui_event.clone(), chapters_boundaries),
            export_ctrl: ExportController::new(&builder, ui_event.clone()),
            split_ctrl: SplitController::new(&builder, ui_event.clone()),
            streams_ctrl: StreamsController::new(&builder),

            pipeline: None,
            missing_plugins: HashSet::<String>::new(),
            state: ControllerState::Stopped,

            new_media_event_handler: None,
            media_event_abort_handle: None,
            callback_when_paused: None,
        }));

        ui_event_handler.have_main_ctrl(&main_ctrl_rc);
        ui_event_handler.spawn();

        let mut main_ctrl = main_ctrl_rc.borrow_mut();
        MainDispatcher::setup(&mut main_ctrl, &main_ctrl_rc, app);

        if gst_init_res.is_ok() {
            {
                let config = CONFIG.read().unwrap();
                if config.ui.width > 0 && config.ui.height > 0 {
                    main_ctrl.window.resize(config.ui.width, config.ui.height);
                    main_ctrl.playback_paned.set_position(config.ui.paned_pos);
                }

                main_ctrl.open_btn.set_sensitive(true);
            }

            ui_event.show_all();

            if let Some(input_file) = args.input_file.to_owned() {
                main_ctrl.ui_event.open_media(input_file);
            }
        } else {
            ui_event.show_all();
        }
    }

    pub fn ui_event(&self) -> &UIEventSender {
        &self.ui_event
    }

    pub fn about(&self) {
        let dialog = gtk::AboutDialog::new();
        dialog.set_modal(true);
        dialog.set_transient_for(Some(&self.window));

        dialog.set_program_name(env!("CARGO_PKG_NAME"));
        dialog.set_logo_icon_name(Some(&APP_ID));
        dialog.set_comments(Some(&gettext(
            "Build a table of contents from a media file\nor split a media file into chapters",
        )));
        dialog.set_copyright(Some(&gettext("© 2017–2020 François Laignel")));
        dialog.set_translator_credits(Some(&gettext("translator-credits")));
        dialog.set_license_type(gtk::License::MitX11);
        dialog.set_version(Some(env!("CARGO_PKG_VERSION")));
        dialog.set_website(Some(env!("CARGO_PKG_HOMEPAGE")));
        dialog.set_website_label(Some(&gettext("Learn more about media-toc")));

        dialog.connect_response(|dialog, _| dialog.close());
        dialog.show();
    }

    pub fn quit(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            pipeline.stop();
        }
        self.abort_media_event_handler();

        self.export_ctrl.cancel();
        self.split_ctrl.cancel();

        {
            let size = self.window.get_size();
            let paned_pos = self.playback_paned.get_position();
            let mut config = CONFIG.write().unwrap();
            config.ui.width = size.0;
            config.ui.height = size.1;
            config.ui.paned_pos = paned_pos;
            config.save();
        }

        self.window.close();
    }

    pub fn play_pause(&mut self) {
        let pipeline = match self.pipeline.as_mut() {
            Some(pipeline) => pipeline,
            None => {
                self.select_media();
                return;
            }
        };

        if self.state != ControllerState::EOS {
            match pipeline.state() {
                gst::State::Paused => {
                    self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                    self.state = ControllerState::Playing;
                    self.audio_ctrl.switch_to_playing();
                    pipeline.play().unwrap();
                }
                gst::State::Playing => {
                    pipeline.pause().unwrap();
                    self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                    self.state = ControllerState::Paused;
                    self.audio_ctrl.switch_to_not_playing();
                }
                _ => {
                    self.select_media();
                }
            };
        } else {
            // Restart the stream from the begining
            self.seek(Timestamp::default(), gst::SeekFlags::ACCURATE);
        }
    }

    pub fn move_chapter_boundary(
        &mut self,
        boundary: Timestamp,
        target: Timestamp,
    ) -> PositionStatus {
        self.info_ctrl.move_chapter_boundary(boundary, target)
    }

    pub fn seek(&mut self, mut ts: Timestamp, mut flags: gst::SeekFlags) {
        let mut must_update_info = false;
        self.state = match self.state {
            ControllerState::Playing => ControllerState::Seeking,
            ControllerState::Paused | ControllerState::PlayingRange(_) => {
                if let ControllerState::PlayingRange(_) = self.state {
                    self.pipeline.as_ref().unwrap().pause().unwrap();
                    self.audio_ctrl.stop_play_range();
                }

                flags = gst::SeekFlags::ACCURATE;
                let seek_1st_step = self.audio_ctrl.first_ts_for_paused_seek(ts);
                match seek_1st_step {
                    Some(seek_1st_step) => {
                        let seek_2s_step = ts;
                        ts = seek_1st_step;
                        ControllerState::TwoStepsSeek(seek_2s_step)
                    }
                    None => {
                        must_update_info = true;
                        ControllerState::Seeking
                    }
                }
            }
            ControllerState::TwoStepsSeek(target) => {
                // seeked position and target might be different if the user
                // seeks repeatedly and rapidly: we can receive a new seek while still
                // being in the `TwoStepsSeek` step from previous seek.
                // Currently, I think it is better to favor completing the in-progress
                // `TwoStepsSeek` (which purpose is to center the cursor on the waveform)
                // than reaching for the latest seeked position
                ts = target;
                must_update_info = true;
                ControllerState::Seeking
            }
            ControllerState::EOS => {
                self.audio_ctrl.switch_to_playing();
                ControllerState::Seeking
            }
            _ => return,
        };

        debug!("seek {} {:?}", ts, self.state);

        if must_update_info {
            self.info_ctrl.seek(ts);
        }

        self.audio_ctrl.seek(ts);

        self.pipeline.as_ref().unwrap().seek(ts, flags);
    }

    pub fn play_range(&mut self, start: Timestamp, end: Timestamp, to_restore: Timestamp) {
        match self.state {
            ControllerState::Paused | ControllerState::PlayingRange(_) => {
                self.audio_ctrl.start_play_range(to_restore);

                self.state = ControllerState::PlayingRange(to_restore);
                self.pipeline.as_ref().unwrap().seek_range(start, end);
            }
            _ => (),
        }
    }

    pub fn current_ts(&mut self) -> Option<Timestamp> {
        self.pipeline.as_mut().unwrap().current_ts()
    }

    pub fn refresh(&mut self) {
        self.audio_ctrl.redraw();
    }

    pub fn refresh_info(&mut self, ts: Timestamp) {
        match self.state {
            ControllerState::Seeking => (),
            _ => self.info_ctrl.tick(ts, self.state),
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
        let info = self.pipeline.as_ref().unwrap().info.read().unwrap();
        self.audio_ctrl.streams_changed(&info);
        self.export_ctrl.streams_changed(&info);
        self.info_ctrl.streams_changed(&info);
        self.perspective_ctrl.streams_changed(&info);
        self.split_ctrl.streams_changed(&info);
        self.video_ctrl.streams_changed(&info);
    }

    pub fn hold(&mut self) {
        self.ui_event.set_cursor_waiting();
        self.audio_ctrl.switch_to_not_playing();
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.pause().unwrap();
        };
    }

    pub fn pause_and_callback(&mut self, callback: Box<dyn Fn(&mut MainController)>) {
        self.audio_ctrl.switch_to_not_playing();
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.pause().unwrap();
        };

        match &self.state {
            ControllerState::Playing | ControllerState::EOS => {
                self.callback_when_paused = Some(callback);
                self.state = ControllerState::PendingPaused;
            }
            ControllerState::Paused => callback(self),
            other => unimplemented!("MainController::pause_and_callback in {:?}", other),
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

    fn spawn_media_event_handler(&mut self) -> async_mpsc::Sender<MediaEvent> {
        let (sender, receiver) = async_mpsc::channel(1);

        let (abortable_event_handler, abort_handle) =
            abortable(self.new_media_event_handler.as_ref().unwrap()(receiver));
        spawn!(abortable_event_handler.map(drop));
        self.media_event_abort_handle = Some(abort_handle);

        sender
    }

    fn abort_media_event_handler(&mut self) {
        if let Some(abort_handle) = self.media_event_abort_handle.take() {
            abort_handle.abort();
        }
    }

    pub fn handle_media_event(&mut self, event: MediaEvent) -> Result<(), ()> {
        let mut keep_going = true;

        match event {
            MediaEvent::AsyncDone(playback_state) => {
                if let ControllerState::Seeking = self.state {
                    self.state = match playback_state {
                        PlaybackState::Playing => ControllerState::Playing,
                        PlaybackState::Paused => ControllerState::Paused,
                    };

                    self.audio_ctrl.seek_complete();
                }
            }
            MediaEvent::InitDone => {
                debug!("received `InitDone`");
                {
                    let pipeline = self.pipeline.as_ref().unwrap();

                    self.header_bar
                        .set_subtitle(Some(pipeline.info.read().unwrap().file_name.as_str()));

                    self.audio_ctrl.new_media(&pipeline);
                    self.export_ctrl.new_media(&pipeline);
                    self.info_ctrl.new_media(&pipeline);
                    self.perspective_ctrl.new_media(&pipeline);
                    self.split_ctrl.new_media(&pipeline);
                    self.streams_ctrl.new_media(&pipeline);
                    self.video_ctrl.new_media(&pipeline);
                }

                self.streams_selected();

                if let Some(message) = self.check_missing_plugins() {
                    self.ui_event.show_error(message);
                }

                self.audio_ctrl.switch_to_not_playing();
                self.ui_event.reset_cursor();
                self.state = ControllerState::Paused;
            }
            MediaEvent::MissingPlugin(plugin) => {
                error!(
                    "{}",
                    gettext("Missing plugin: {}").replacen("{}", &plugin, 1)
                );
                self.missing_plugins.insert(plugin);
            }
            MediaEvent::ReadyToRefresh => match &self.state {
                ControllerState::Playing => (),
                ControllerState::Paused => {
                    self.refresh();
                }
                ControllerState::TwoStepsSeek(target) => {
                    let target = *target; // let go the reference on `self.state`
                    self.seek(target, gst::SeekFlags::ACCURATE);
                }
                ControllerState::PendingSelectMedia => {
                    self.select_media();
                }
                ControllerState::PendingPaused => {
                    self.state = ControllerState::Paused;
                    if let Some(callback) = self.callback_when_paused.take() {
                        callback(self)
                    }
                }
                _ => (),
            },
            MediaEvent::StreamsSelected => self.streams_selected(),
            MediaEvent::Eos => {
                match self.state {
                    ControllerState::PlayingRange(pos_to_restore) => {
                        // end of range => pause and seek back to pos_to_restore
                        self.pipeline.as_ref().unwrap().pause().unwrap();
                        self.state = ControllerState::Paused;
                        self.audio_ctrl.stop_play_range();
                        self.seek(pos_to_restore, gst::SeekFlags::ACCURATE);
                    }
                    _ => {
                        self.state = ControllerState::EOS;
                        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                        self.audio_ctrl.switch_to_not_playing();
                    }
                }
            }
            MediaEvent::FailedToOpenMedia(error) => {
                self.pipeline = None;
                self.state = ControllerState::Stopped;
                self.ui_event.reset_cursor();

                keep_going = false;

                let mut error = gettext("Error opening file.\n\n{}").replacen("{}", &error, 1);
                if let Some(message) = self.check_missing_plugins() {
                    error += "\n\n";
                    error += &message;
                }
                self.ui_event.show_error(error);
            }
            MediaEvent::GLSinkError => {
                self.pipeline = None;
                self.state = ControllerState::Stopped;
                self.ui_event.reset_cursor();

                let mut config = CONFIG.write().expect("Failed to get CONFIG as mut");
                config.media.is_gl_disabled = true;
                config.save();

                keep_going = false;

                self.ui_event.show_error(gettext(
"Video rendering hardware acceleration seems broken and has been disabled.\nPlease restart the application.",
                ));
            }
            _ => (),
        }

        if keep_going {
            Ok(())
        } else {
            self.audio_ctrl.switch_to_not_playing();
            Err(())
        }
    }

    pub fn select_media(&mut self) {
        self.state = ControllerState::PendingSelectMediaDecision;
        self.ui_event.hide_info_bar();

        if let Some(ref last_path) = CONFIG.read().unwrap().media.last_path {
            self.file_dlg.set_current_folder(last_path);
        }
        self.file_dlg.show();
    }

    pub fn open_media(&mut self, path: PathBuf) {
        if let Some(pipeline) = self.pipeline.take() {
            pipeline.stop();
        }

        self.abort_media_event_handler();

        self.info_ctrl.cleanup();
        self.audio_ctrl.cleanup();
        self.video_ctrl.cleanup();
        self.export_ctrl.cleanup();
        self.split_ctrl.cleanup();
        self.streams_ctrl.cleanup();
        self.perspective_ctrl.cleanup();
        self.header_bar.set_subtitle(Some(""));

        self.state = ControllerState::Stopped;
        self.missing_plugins.clear();
        let sender = self.spawn_media_event_handler();

        let dbl_buffer_mtx = Arc::clone(&self.audio_ctrl.dbl_renderer_mtx);
        match PlaybackPipeline::try_new(
            path.as_ref(),
            &dbl_buffer_mtx,
            &self.video_ctrl.video_sink(),
            sender,
        ) {
            Ok(pipeline) => {
                CONFIG.write().unwrap().media.last_path = path.parent().map(ToOwned::to_owned);
                self.pipeline = Some(pipeline);
            }
            Err(error) => {
                self.ui_event.reset_cursor();
                let error = gettext("Error opening file.\n\n{}").replace("{}", &error);
                self.ui_event.show_error(error);
            }
        };
    }

    pub fn cancel_select_media(&mut self) {
        if self.state == ControllerState::PendingSelectMediaDecision {
            self.state = self
                .pipeline
                .as_ref()
                .map_or(ControllerState::Stopped, |_| ControllerState::Paused);
        }
    }
}
