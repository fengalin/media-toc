use futures::channel::mpsc as async_mpsc;
use futures::future::{abortable, AbortHandle, LocalBoxFuture};
use futures::prelude::*;

use gettextrs::{gettext, ngettext};
use gtk::prelude::*;

use log::{debug, error};

use std::{borrow::ToOwned, cell::RefCell, collections::HashSet, path::PathBuf, rc::Rc, sync::Arc};

use application::{CommandLineArguments, APP_ID, APP_PATH, CONFIG};
use media::{MediaEvent, PlaybackState, Timestamp};

use super::{
    spawn, ui_event, AudioAreaEvent, AudioController, ChapterEntry, ChaptersBoundaries,
    ExportController, InfoController, MainDispatcher, MediaEventReceiver, PerspectiveController,
    PlaybackPipeline, PositionStatus, SplitController, StreamsController, UIController,
    UIEventSender, VideoController,
};

const PAUSE_ICON: &str = "media-playback-pause-symbolic";
const PLAYBACK_ICON: &str = "media-playback-start-symbolic";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControllerState {
    CompletePlayRange(Timestamp),
    EOS,
    Paused,
    PausedPlayingRange(Timestamp),
    PendingPaused,
    PendingSelectMedia,
    PendingSelectMediaDecision,
    PendingSeek(Timestamp),
    Playing,
    PlayingRange(Timestamp),
    Seeking(Timestamp),
    Stopped,
    TwoStepsSeek(Timestamp),
}

pub struct MainController {
    pub(super) window: gtk::ApplicationWindow,
    pub(super) window_delete_id: Option<glib::signal::SignalHandlerId>,

    header_bar: gtk::HeaderBar,
    pub(super) open_btn: gtk::Button,
    pub(super) display_page: gtk::Box,
    playback_paned: gtk::Paned,
    pub(super) play_pause_btn: gtk::ToolButton,
    file_dlg: gtk::FileChooserNative,

    pub(super) ui_event: UIEventSender,

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

        let (ui_event, ui_event_receiver) = ui_event::new_pair();
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
            window: window.clone(),
            window_delete_id: None,

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

        let mut main_ctrl = main_ctrl_rc.borrow_mut();
        MainDispatcher::setup(
            &mut main_ctrl,
            &main_ctrl_rc,
            app,
            &window,
            &builder,
            ui_event_receiver,
        );

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

        if let Some(window_delete_id) = self.window_delete_id.take() {
            let size = self.window.get_size();
            let paned_pos = self.playback_paned.get_position();
            let mut config = CONFIG.write().unwrap();
            config.ui.width = size.0;
            config.ui.height = size.1;
            config.ui.paned_pos = paned_pos;
            config.save();

            // Restore default delete handler
            glib::signal::signal_handler_disconnect(&self.window, window_delete_id);
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

        match self.state {
            ControllerState::Paused => {
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                self.state = ControllerState::Playing;
                self.audio_ctrl.play();
                pipeline.play().unwrap();
            }
            ControllerState::PausedPlayingRange(to_restore) => {
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                self.state = ControllerState::PlayingRange(to_restore);
                self.audio_ctrl.play();
                pipeline.play().unwrap();
            }
            ControllerState::PlayingRange(to_restore) => {
                self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                self.state = ControllerState::PausedPlayingRange(to_restore);
                pipeline.pause().unwrap();
                self.audio_ctrl.pause();
            }
            ControllerState::Playing => {
                self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                self.state = ControllerState::Paused;
                self.audio_ctrl.pause();
                pipeline.pause().unwrap();
            }
            ControllerState::EOS => {
                // Restart the stream from the begining
                self.seek(Timestamp::default(), gst::SeekFlags::ACCURATE);
            }
            ControllerState::Stopped => self.select_media(),
            _ => (),
        }
    }

    pub fn seek(&mut self, mut ts: Timestamp, mut flags: gst::SeekFlags) {
        match self.state {
            ControllerState::Playing => {
                self.state = ControllerState::Seeking(ts);
                self.audio_ctrl.seek();
            }
            ControllerState::Paused | ControllerState::PausedPlayingRange(_) => {
                flags = gst::SeekFlags::ACCURATE;
                let seek_1st_step = self.audio_ctrl.first_ts_for_paused_seek(ts);
                match seek_1st_step {
                    Some(seek_1st_step) => {
                        let seek_2d_step = ts;
                        ts = seek_1st_step;
                        self.state = ControllerState::TwoStepsSeek(seek_2d_step);
                        self.audio_ctrl.seek();
                    }
                    None => {
                        self.state = ControllerState::Seeking(ts);
                        self.audio_ctrl.seek();
                    }
                }
            }
            ControllerState::PlayingRange(_) => {
                self.state = ControllerState::PendingSeek(ts);
                self.pipeline.as_ref().unwrap().pause().unwrap();
                return;
            }
            ControllerState::TwoStepsSeek(target) => {
                // seeked position and target might be different if the user
                // seeks repeatedly and rapidly: we can receive a new seek while still
                // being in the `TwoStepsSeek` step from previous seek.
                // Currently, I think it is better to favor completing the in-progress
                // `TwoStepsSeek` (which purpose is to center the cursor on the waveform)
                // than reaching for the latest seeked position
                ts = target;
                self.state = ControllerState::Seeking(ts);
            }
            ControllerState::EOS => {
                self.state = ControllerState::Seeking(ts);
                self.audio_ctrl.play();
                self.audio_ctrl.seek();
            }
            _ => return,
        }

        debug!("triggerging seek {} {:?}", ts, self.state);

        self.pipeline.as_ref().unwrap().seek(ts, flags);
    }

    pub fn play_range(&mut self, start: Timestamp, end: Timestamp, to_restore: Timestamp) {
        match self.state {
            ControllerState::Paused
            | ControllerState::PlayingRange(_)
            | ControllerState::PausedPlayingRange(_) => {
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
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

    pub fn redraw(&mut self) {
        self.audio_ctrl.redraw();
    }

    pub fn refresh_info(&mut self, ts: Timestamp) {
        match self.state {
            ControllerState::Seeking(_) => (),
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
        self.audio_ctrl.pause();
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.pause().unwrap();
        };
    }

    pub fn pause_and_callback(&mut self, callback: Box<dyn Fn(&mut MainController)>) {
        self.audio_ctrl.pause();
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
        spawn(abortable_event_handler.map(drop));
        self.media_event_abort_handle = Some(abort_handle);

        sender
    }

    fn abort_media_event_handler(&mut self) {
        if let Some(abort_handle) = self.media_event_abort_handle.take() {
            abort_handle.abort();
        }
    }

    pub fn handle_media_event(&mut self, event: MediaEvent) -> Result<(), ()> {
        match self.state {
            ControllerState::Playing => match event {
                MediaEvent::Eos => {
                    self.state = ControllerState::EOS;
                    self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                    self.audio_ctrl.pause();
                }
                // FIXME select_stream might be eligible for an async impl
                MediaEvent::StreamsSelected => self.streams_selected(),
                _ => (),
            },
            ControllerState::PlayingRange(pos_to_restore) => match event {
                MediaEvent::Eos => {
                    // end of range => pause and seek back to pos_to_restore
                    self.pipeline.as_ref().unwrap().pause().unwrap();
                    self.state = ControllerState::CompletePlayRange(pos_to_restore);
                }
                // FIXME select_stream might be eligible for an async impl
                MediaEvent::StreamsSelected => self.streams_selected(),
                _ => (),
            },
            ControllerState::CompletePlayRange(pos_to_restore) => match event {
                MediaEvent::ReadyToRefresh => {
                    self.pipeline
                        .as_ref()
                        .unwrap()
                        .seek(pos_to_restore, gst::SeekFlags::ACCURATE);
                }
                MediaEvent::AsyncDone(_) => {
                    self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                    self.state = ControllerState::Paused;
                    self.audio_ctrl.stop_play_range();
                }
                _ => (),
            },
            ControllerState::Seeking(ts) => {
                if let MediaEvent::AsyncDone(playback_state) = event {
                    match playback_state {
                        PlaybackState::Playing => self.state = ControllerState::Playing,
                        PlaybackState::Paused => self.state = ControllerState::Paused,
                    }

                    debug!("seek to {} done", ts);
                    self.info_ctrl.seek_done(ts);
                    self.audio_ctrl.seek_done(ts);
                }
            }
            ControllerState::TwoStepsSeek(target) => {
                if let MediaEvent::ReadyToRefresh = event {
                    self.seek(target, gst::SeekFlags::ACCURATE);
                }
            }
            ControllerState::Paused => match event {
                MediaEvent::ReadyToRefresh => self.audio_ctrl.refresh(),
                // FIXME select_stream might be eligible for an async impl
                MediaEvent::StreamsSelected => self.streams_selected(),
                _ => (),
            },
            ControllerState::EOS | ControllerState::PausedPlayingRange(_) => {
                if let MediaEvent::StreamsSelected = event {
                    // FIXME select_stream might be eligible for an async impl
                    self.streams_selected();
                }
            }
            // FIXME Pending states seem to be eligible for async implementations
            ControllerState::PendingSeek(ts) => {
                if let MediaEvent::ReadyToRefresh = event {
                    self.state = ControllerState::Paused;
                    self.seek(ts, gst::SeekFlags::ACCURATE);
                }
            }
            ControllerState::PendingPaused => {
                if let MediaEvent::ReadyToRefresh = event {
                    self.state = ControllerState::Paused;
                    if let Some(callback) = self.callback_when_paused.take() {
                        callback(self)
                    }
                }
            }
            ControllerState::PendingSelectMedia => {
                if let MediaEvent::ReadyToRefresh = event {
                    self.select_media();
                }
            }
            ControllerState::Stopped | ControllerState::PendingSelectMediaDecision => match event {
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

                    self.audio_ctrl.pause();
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
                MediaEvent::FailedToOpenMedia(error) => {
                    self.pipeline = None;
                    self.state = ControllerState::Stopped;
                    self.ui_event.reset_cursor();

                    let mut error = gettext("Error opening file.\n\n{}").replacen("{}", &error, 1);
                    if let Some(message) = self.check_missing_plugins() {
                        error += "\n\n";
                        error += &message;
                    }
                    self.ui_event.show_error(error);

                    return Err(());
                }
                MediaEvent::GLSinkError => {
                    self.pipeline = None;
                    self.state = ControllerState::Stopped;
                    self.ui_event.reset_cursor();

                    let mut config = CONFIG.write().expect("Failed to get CONFIG as mut");
                    config.media.is_gl_disabled = true;
                    config.save();

                    self.ui_event.show_error(gettext(
    "Video rendering hardware acceleration seems broken and has been disabled.\nPlease restart the application.",
                    ));

                    return Err(());
                }
                _ => (),
            },
        }

        Ok(())
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
            self.state = if self.pipeline.is_some() {
                ControllerState::Paused
            } else {
                ControllerState::Stopped
            };
        }
    }

    pub fn chapter_clicked(&mut self, chapter_path: gtk::TreePath) {
        let seek_ts = self
            .info_ctrl
            .chapter_manager
            .chapter_from_path(&chapter_path)
            .as_ref()
            .map(ChapterEntry::start);

        if let Some(seek_ts) = seek_ts {
            let _ = self.seek(seek_ts, gst::SeekFlags::ACCURATE);
        }
    }

    pub fn next_chapter(&mut self) {
        let seek_ts = self
            .info_ctrl
            .chapter_manager
            .pick_next()
            .as_ref()
            .map(ChapterEntry::start);

        if let Some(seek_ts) = seek_ts {
            let _ = self.seek(seek_ts, gst::SeekFlags::ACCURATE);
        }
    }

    pub fn previous_chapter(&mut self) {
        let seek_ts = self
            .current_ts()
            .and_then(|cur_ts| self.info_ctrl.previous_chapter(cur_ts));

        let _ = self.seek(
            seek_ts.unwrap_or_else(Timestamp::default),
            gst::SeekFlags::ACCURATE,
        );
    }

    pub fn step_back(&mut self) {
        if let Some(current_ts) = self.current_ts() {
            let seek_ts = {
                let seek_step = self.audio_ctrl.seek_step;
                if current_ts > seek_step {
                    current_ts - seek_step
                } else {
                    Timestamp::default()
                }
            };
            let _ = self.seek(seek_ts, gst::SeekFlags::ACCURATE);
        }
    }

    pub fn step_forward(&mut self) {
        if let Some(current_ts) = self.current_ts() {
            let seek_ts = current_ts + self.audio_ctrl.seek_step;
            let _ = self.seek(seek_ts, gst::SeekFlags::ACCURATE);
        }
    }

    pub fn stream_clicked(&mut self, type_: gst::StreamType) {
        if let super::StreamClickedStatus::Changed = self.streams_ctrl.stream_clicked(type_) {
            let streams = self.streams_ctrl.selected_streams();
            self.select_streams(&streams);
        }
    }

    pub fn stream_export_toggled(&mut self, type_: gst::StreamType, tree_path: gtk::TreePath) {
        if let Some((stream_id, must_export)) = self.streams_ctrl.export_toggled(type_, tree_path) {
            if let Some(pipeline) = self.pipeline.as_mut() {
                pipeline
                    .info
                    .write()
                    .unwrap()
                    .streams
                    .collection_mut(type_)
                    .get_mut(stream_id)
                    .as_mut()
                    .unwrap()
                    .must_export = must_export;
            }
        }
    }

    pub fn audio_area_event(&mut self, event: AudioAreaEvent) {
        match event {
            AudioAreaEvent::Button(event) => match event.get_event_type() {
                gdk::EventType::ButtonPress => self.audio_ctrl.button_pressed(event),
                gdk::EventType::ButtonRelease => self.audio_ctrl.button_released(event),
                gdk::EventType::Scroll => {
                    // FIXME zoom in / out
                }
                _ => (),
            },
            AudioAreaEvent::Leaving => self.audio_ctrl.leave_drawing_area(),
            AudioAreaEvent::Motion(event) => {
                if let Some((boundary, target)) = self.audio_ctrl.motion_notify(event) {
                    if let PositionStatus::ChapterChanged { .. } =
                        self.info_ctrl.move_chapter_boundary(boundary, target)
                    {
                        // FIXME this is ugly
                        self.audio_ctrl.state =
                            super::audio_controller::ControllerState::MovingBoundary(target);
                        self.audio_ctrl.drawingarea.queue_draw();
                    }
                }
            }
        }
    }

    pub fn add_chapter(&mut self) {
        if let Some(ts) = self.current_ts() {
            self.info_ctrl.add_chapter(ts);
        }
    }

    pub fn remove_chapter(&mut self) {
        self.info_ctrl.remove_chapter();
    }

    pub fn rename_chapter(&mut self, new_title: &str) {
        self.info_ctrl.chapter_manager.rename_selected(new_title);
        // reflect title modification in other parts of the UI (audio waveform)
        self.redraw();
    }
}
