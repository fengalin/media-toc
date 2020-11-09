use futures::channel::mpsc as async_mpsc;

use gettextrs::{gettext, ngettext};
use gtk::prelude::*;

use log::{debug, error};

use std::{borrow::ToOwned, cell::RefCell, collections::HashSet, path::PathBuf, rc::Rc, sync::Arc};

use application::{CommandLineArguments, APP_ID, CONFIG};
use media::{MediaEvent, PlaybackState, Timestamp};

use crate::{
    audio, export,
    info::{self, ChaptersBoundaries},
    info_bar, main, perspective,
    prelude::*,
    split, streams, video,
};

const PAUSE_ICON: &str = "media-playback-pause-symbolic";
const PLAYBACK_ICON: &str = "media-playback-start-symbolic";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
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

pub struct Controller {
    window: gtk::ApplicationWindow,
    pub(super) window_delete_id: Option<glib::signal::SignalHandlerId>,

    header_bar: gtk::HeaderBar,
    pub(super) playback_paned: gtk::Paned,
    pub(crate) play_pause_btn: gtk::ToolButton,
    file_dlg: gtk::FileChooserNative,

    media_event_sender: async_mpsc::Sender<MediaEvent>,

    pub(crate) perspective: perspective::Controller,
    pub(crate) video: video::Controller,
    pub(crate) info: info::Controller,
    pub(crate) info_bar: info_bar::Controller,
    pub(crate) audio: audio::Controller,
    pub(crate) export: export::Controller,
    pub(crate) split: split::Controller,
    pub(crate) streams: streams::Controller,

    pub(crate) pipeline: Option<PlaybackPipeline>,
    missing_plugins: HashSet<String>,
    pub(crate) state: State,

    callback_when_paused: Option<Box<dyn Fn(&mut main::Controller)>>,
}

impl Controller {
    pub fn new(
        window: &gtk::ApplicationWindow,
        args: &CommandLineArguments,
        builder: &gtk::Builder,
        media_event_sender: async_mpsc::Sender<MediaEvent>,
    ) -> Self {
        let chapters_boundaries = Rc::new(RefCell::new(ChaptersBoundaries::new()));

        let file_dlg = gtk::FileChooserNativeBuilder::new()
            .title(&gettext("Open a media file"))
            .transient_for(window)
            .modal(true)
            .accept_label(&gettext("Open"))
            .cancel_label(&gettext("Cancel"))
            .build();

        file_dlg.connect_response(|file_dlg, response| {
            file_dlg.hide();
            match (response, file_dlg.get_filename()) {
                (gtk::ResponseType::Accept, Some(path)) => main::open_media(path),
                _ => main::cancel_select_media(),
            }
        });

        Controller {
            window: window.clone(),
            window_delete_id: None,

            header_bar: builder.get_object("header-bar").unwrap(),
            playback_paned: builder.get_object("playback-paned").unwrap(),
            play_pause_btn: builder.get_object("play_pause-toolbutton").unwrap(),
            file_dlg,

            media_event_sender,

            perspective: perspective::Controller::new(builder),
            video: video::Controller::new(builder, args),
            info: info::Controller::new(&builder, Rc::clone(&chapters_boundaries)),
            info_bar: info_bar::Controller::new(builder),
            audio: audio::Controller::new(builder, chapters_boundaries),
            export: export::Controller::new(builder),
            split: split::Controller::new(builder),
            streams: streams::Controller::new(builder),

            pipeline: None,
            missing_plugins: HashSet::<String>::new(),
            state: State::Stopped,

            callback_when_paused: None,
        }
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

        self.export.cancel();
        self.split.cancel();

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
            State::Paused => {
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                self.state = State::Playing;
                self.audio.play();
                pipeline.play().unwrap();
            }
            State::PausedPlayingRange(to_restore) => {
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                self.state = State::PlayingRange(to_restore);
                self.audio.play();
                pipeline.play().unwrap();
            }
            State::PlayingRange(to_restore) => {
                self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                self.state = State::PausedPlayingRange(to_restore);
                pipeline.pause().unwrap();
                self.audio.pause();
            }
            State::Playing => {
                self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                self.state = State::Paused;
                self.audio.pause();
                pipeline.pause().unwrap();
            }
            State::EOS => {
                // Restart the stream from the begining
                self.seek(Timestamp::default(), gst::SeekFlags::ACCURATE);
            }
            State::Stopped => self.select_media(),
            _ => (),
        }
    }

    pub fn seek(&mut self, mut ts: Timestamp, mut flags: gst::SeekFlags) {
        match self.state {
            State::Playing => {
                self.state = State::Seeking(ts);
                self.audio.seek();
            }
            State::Paused | State::PausedPlayingRange(_) => {
                flags = gst::SeekFlags::ACCURATE;
                let seek_1st_step = self.audio.first_ts_for_paused_seek(ts);
                match seek_1st_step {
                    Some(seek_1st_step) => {
                        let seek_2d_step = ts;
                        ts = seek_1st_step;
                        self.state = State::TwoStepsSeek(seek_2d_step);
                        self.audio.seek();
                    }
                    None => {
                        self.state = State::Seeking(ts);
                        self.audio.seek();
                    }
                }
            }
            State::PlayingRange(_) => {
                self.state = State::PendingSeek(ts);
                self.pipeline.as_ref().unwrap().pause().unwrap();
                return;
            }
            State::TwoStepsSeek(target) => {
                // seeked position and target might be different if the user
                // seeks repeatedly and rapidly: we can receive a new seek while still
                // being in the `TwoStepsSeek` step from previous seek.
                // Currently, I think it is better to favor completing the in-progress
                // `TwoStepsSeek` (which purpose is to center the cursor on the waveform)
                // than reaching for the latest seeked position
                ts = target;
                self.state = State::Seeking(ts);
            }
            State::EOS => {
                self.state = State::Seeking(ts);
                self.audio.play();
                self.audio.seek();
            }
            _ => return,
        }

        debug!("triggerging seek {} {:?}", ts, self.state);

        self.pipeline.as_ref().unwrap().seek(ts, flags);
    }

    pub fn play_range(&mut self, start: Timestamp, end: Timestamp, to_restore: Timestamp) {
        match self.state {
            State::Paused | State::PlayingRange(_) | State::PausedPlayingRange(_) => {
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                self.audio.start_play_range(to_restore);

                self.state = State::PlayingRange(to_restore);
                self.pipeline.as_ref().unwrap().seek_range(start, end);
            }
            _ => (),
        }
    }

    pub fn current_ts(&mut self) -> Option<Timestamp> {
        self.pipeline.as_mut().unwrap().current_ts()
    }

    pub fn redraw(&mut self) {
        self.audio.redraw();
    }

    pub fn select_streams(&mut self, stream_ids: &[Arc<str>]) {
        self.pipeline.as_ref().unwrap().select_streams(stream_ids);
        // In Playing state, wait for the notification from the pipeline
        // Otherwise, update immediately
        if self.state != State::Playing {
            self.streams_selected();
        }
    }

    pub fn streams_selected(&mut self) {
        let info = self.pipeline.as_ref().unwrap().info.read().unwrap();
        self.audio.streams_changed(&info);
        self.export.streams_changed(&info);
        self.info.streams_changed(&info);
        self.perspective.streams_changed(&info);
        self.split.streams_changed(&info);
        self.video.streams_changed(&info);
    }

    pub fn hold(&mut self) {
        main::set_cursor_waiting();
        self.audio.pause();
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.pause().unwrap();
        };
    }

    pub fn pause_and_callback(&mut self, callback: Box<dyn Fn(&mut main::Controller)>) {
        self.audio.pause();
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.pause().unwrap();
        };

        match &self.state {
            State::Playing | State::EOS => {
                self.callback_when_paused = Some(callback);
                self.state = State::PendingPaused;
            }
            State::Paused => callback(self),
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

    pub fn handle_media_event(&mut self, event: MediaEvent) {
        match self.state {
            State::Playing => match event {
                MediaEvent::Eos => {
                    self.state = State::EOS;
                    self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                    self.audio.pause();
                }
                // FIXME select_stream might be eligible for an async impl
                MediaEvent::StreamsSelected => self.streams_selected(),
                _ => (),
            },
            State::PlayingRange(pos_to_restore) => match event {
                MediaEvent::Eos => {
                    // end of range => pause and seek back to pos_to_restore
                    self.pipeline.as_ref().unwrap().pause().unwrap();
                    self.state = State::CompletePlayRange(pos_to_restore);
                }
                // FIXME select_stream might be eligible for an async impl
                MediaEvent::StreamsSelected => self.streams_selected(),
                _ => (),
            },
            State::CompletePlayRange(pos_to_restore) => match event {
                MediaEvent::ReadyToRefresh => {
                    self.pipeline
                        .as_ref()
                        .unwrap()
                        .seek(pos_to_restore, gst::SeekFlags::ACCURATE);
                }
                MediaEvent::AsyncDone(_) => {
                    self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                    self.state = State::Paused;
                    self.audio.stop_play_range();
                }
                _ => (),
            },
            State::Seeking(ts) => {
                if let MediaEvent::AsyncDone(playback_state) = event {
                    match playback_state {
                        PlaybackState::Playing => self.state = State::Playing,
                        PlaybackState::Paused => self.state = State::Paused,
                    }

                    debug!("seek to {} done", ts);
                    self.info.seek_done(ts);
                    self.audio.seek_done(ts);
                }
            }
            State::TwoStepsSeek(target) => {
                if let MediaEvent::ReadyToRefresh = event {
                    self.seek(target, gst::SeekFlags::ACCURATE);
                }
            }
            State::Paused => match event {
                MediaEvent::ReadyToRefresh => self.audio.refresh(),
                // FIXME select_stream might be eligible for an async impl
                MediaEvent::StreamsSelected => self.streams_selected(),
                _ => (),
            },
            State::EOS | State::PausedPlayingRange(_) => {
                if let MediaEvent::StreamsSelected = event {
                    // FIXME select_stream might be eligible for an async impl
                    self.streams_selected();
                }
            }
            // FIXME Pending states seem to be eligible for async implementations
            State::PendingSeek(ts) => {
                if let MediaEvent::ReadyToRefresh = event {
                    self.state = State::Paused;
                    self.seek(ts, gst::SeekFlags::ACCURATE);
                }
            }
            State::PendingPaused => {
                if let MediaEvent::ReadyToRefresh = event {
                    self.state = State::Paused;
                    if let Some(callback) = self.callback_when_paused.take() {
                        callback(self)
                    }
                }
            }
            State::PendingSelectMedia => {
                if let MediaEvent::ReadyToRefresh = event {
                    self.select_media();
                }
            }
            State::Stopped | State::PendingSelectMediaDecision => match event {
                MediaEvent::InitDone => {
                    debug!("received `InitDone`");
                    {
                        let pipeline = self.pipeline.as_ref().unwrap();

                        self.header_bar
                            .set_subtitle(Some(pipeline.info.read().unwrap().file_name.as_str()));

                        self.audio.new_media(&pipeline);
                        self.export.new_media(&pipeline);
                        self.info.new_media(&pipeline);
                        self.perspective.new_media(&pipeline);
                        self.split.new_media(&pipeline);
                        self.streams.new_media(&pipeline);
                        self.video.new_media(&pipeline);
                    }

                    self.streams_selected();

                    if let Some(message) = self.check_missing_plugins() {
                        info_bar::show_error(message);
                    }

                    self.audio.pause();
                    main::reset_cursor();
                    self.state = State::Paused;
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
                    self.state = State::Stopped;
                    main::reset_cursor();

                    let mut error = gettext("Error opening file.\n\n{}").replacen("{}", &error, 1);
                    if let Some(message) = self.check_missing_plugins() {
                        error += "\n\n";
                        error += &message;
                    }
                    info_bar::show_error(error);
                }
                MediaEvent::GLSinkError => {
                    self.pipeline = None;
                    self.state = State::Stopped;
                    main::reset_cursor();

                    let mut config = CONFIG.write().expect("Failed to get CONFIG as mut");
                    config.media.is_gl_disabled = true;
                    config.save();

                    info_bar::show_info(gettext(
    "Video rendering hardware acceleration seems broken and has been disabled.\nPlease restart the application.",
                    ));
                }
                _ => (),
            },
        }
    }

    pub fn select_media(&mut self) {
        match self.state {
            State::Playing | State::EOS => {
                self.hold();
                self.state = State::PendingSelectMedia;
            }
            _ => {
                self.state = State::PendingSelectMediaDecision;
                info_bar::hide();

                if let Some(ref last_path) = CONFIG.read().unwrap().media.last_path {
                    self.file_dlg.set_current_folder(last_path);
                }
                self.file_dlg.show();
            }
        }
    }

    pub fn open_media(&mut self, path: PathBuf) {
        if let Some(pipeline) = self.pipeline.take() {
            pipeline.stop();
        }

        self.info.cleanup();
        self.audio.cleanup();
        self.video.cleanup();
        self.export.cleanup();
        self.split.cleanup();
        self.streams.cleanup();
        self.perspective.cleanup();
        self.header_bar.set_subtitle(Some(""));

        self.state = State::Stopped;
        self.missing_plugins.clear();

        let dbl_buffer_mtx = Arc::clone(&self.audio.dbl_renderer_mtx);
        match PlaybackPipeline::try_new(
            path.as_ref(),
            &dbl_buffer_mtx,
            &self.video.video_sink(),
            self.media_event_sender.clone(),
        ) {
            Ok(pipeline) => {
                CONFIG.write().unwrap().media.last_path = path.parent().map(ToOwned::to_owned);
                self.pipeline = Some(pipeline);
            }
            Err(error) => {
                main::reset_cursor();
                let error = gettext("Error opening file.\n\n{}").replace("{}", &error);
                info_bar::show_error(error);
            }
        };
    }

    pub fn cancel_select_media(&mut self) {
        if self.state == State::PendingSelectMediaDecision {
            self.state = if self.pipeline.is_some() {
                State::Paused
            } else {
                State::Stopped
            };
        }
    }
}

impl UIController for Controller {
    fn cleanup(&mut self) {}
}
