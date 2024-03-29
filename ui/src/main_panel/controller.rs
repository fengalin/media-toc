use futures::{
    future::{abortable, AbortHandle},
    prelude::*,
};
use gtk::{glib, prelude::*};
use log::{error, info};

use std::{borrow::ToOwned, cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

use application::{gettext, ngettext, CommandLineArguments, APP_ID, CONFIG};
use media::{pipeline, MediaEvent, MissingPlugins, OpenError, SeekError, SelectStreamsError};
use renderers::Timestamp;

use crate::{
    audio, export,
    info::{self, ChaptersBoundaries},
    info_bar, main_panel, perspective, playback,
    prelude::*,
    spawn, split, streams, video,
};

const PAUSE_ICON: &str = "media-playback-pause-symbolic";
const PLAYBACK_ICON: &str = "media-playback-start-symbolic";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    EosPaused,
    EosPlaying,
    Paused,
    PausedPlayingRange,
    PendingSelectMediaDecision,
    Playing,
    PlayingRange,
    Stopped,
}

impl State {
    pub fn is_eos(&self) -> bool {
        matches!(self, State::EosPaused | State::EosPlaying)
    }
}

pub struct Controller {
    window: gtk::ApplicationWindow,
    pub(super) window_delete_id: Option<glib::signal::SignalHandlerId>,

    header_bar: gtk::HeaderBar,
    pub(super) playback_paned: gtk::Paned,
    pub(crate) play_pause_btn: gtk::ToolButton,
    file_dlg: gtk::FileChooserNative,

    pub(crate) perspective: perspective::Controller,
    pub(crate) video: video::Controller,
    pub(crate) info: info::Controller,
    pub(crate) info_bar: info_bar::Controller,
    pub(crate) audio: audio::Controller,
    pub(crate) export: export::Controller,
    pub(crate) split: split::Controller,
    pub(crate) streams: streams::Controller,

    pub(crate) pipeline: Option<pipeline::Playback>,
    pub(crate) state: State,
    pub(crate) seek_manager: playback::SeekManager,

    media_msg_abort_handle: Option<AbortHandle>,
}

impl Controller {
    pub fn new(
        window: &gtk::ApplicationWindow,
        args: &CommandLineArguments,
        builder: &gtk::Builder,
    ) -> Self {
        let chapters_boundaries = Rc::new(RefCell::new(ChaptersBoundaries::new()));

        let file_dlg = gtk::FileChooserNative::builder()
            .title(&gettext("Open a media file"))
            .transient_for(window)
            .modal(true)
            .accept_label(&gettext("Open"))
            .cancel_label(&gettext("Cancel"))
            .build();

        file_dlg.connect_response(|file_dlg, response| {
            file_dlg.hide();
            match (response, file_dlg.filename()) {
                (gtk::ResponseType::Accept, Some(path)) => main_panel::open_media(path),
                _ => main_panel::cancel_select_media(),
            }
        });

        Controller {
            window: window.clone(),
            window_delete_id: None,

            header_bar: builder.object("header-bar").unwrap(),
            playback_paned: builder.object("playback-paned").unwrap(),
            play_pause_btn: builder.object("play_pause-toolbutton").unwrap(),
            file_dlg,

            perspective: perspective::Controller::new(builder),
            video: video::Controller::new(builder, args),
            info: info::Controller::new(builder, Rc::clone(&chapters_boundaries)),
            info_bar: info_bar::Controller::new(builder),
            audio: audio::Controller::new(builder, chapters_boundaries),
            export: export::Controller::new(builder),
            split: split::Controller::new(builder),
            streams: streams::Controller::new(builder),

            pipeline: None,
            state: State::Stopped,
            seek_manager: playback::SeekManager::default(),

            media_msg_abort_handle: None,
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
        dialog.set_copyright(Some(&gettext("© 2017–2022 François Laignel")));
        dialog.set_translator_credits(Some(&gettext("translator-credits")));
        dialog.set_license_type(gtk::License::MitX11);
        dialog.set_version(Some(env!("CARGO_PKG_VERSION")));
        dialog.set_website(Some(env!("CARGO_PKG_HOMEPAGE")));
        dialog.set_website_label(Some(&gettext("Learn more about media-toc")));

        dialog.connect_response(|dialog, _| dialog.close());
        dialog.show();
    }

    pub fn quit(&mut self) {
        if let Some(mut pipeline) = self.pipeline.take() {
            let _ = pipeline.stop();
        }

        self.export.cancel();
        self.split.cancel();

        if let Some(window_delete_id) = self.window_delete_id.take() {
            let size = self.window.size();
            let paned_pos = self.playback_paned.position();
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

    pub async fn play_pause(&mut self) {
        use State::*;

        match self.state {
            Paused => {
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                self.state = Playing;
                self.pipeline.as_mut().unwrap().play().await.unwrap();
                self.audio.play();
            }
            Playing => {
                self.pipeline.as_mut().unwrap().pause().await.unwrap();
                self.audio.pause();
                self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                self.state = Paused;
            }
            EosPlaying | EosPaused => {
                // Restart the stream from the begining
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                self.state = Playing;

                if self
                    .seek(Timestamp::default(), gst::SeekFlags::ACCURATE)
                    .await
                    .is_ok()
                {
                    self.pipeline.as_mut().unwrap().play().await.unwrap();
                    self.audio.play();
                }
            }
            PlayingRange => {
                self.pipeline.as_mut().unwrap().pause().await.unwrap();
                self.audio.pause();
                self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                self.state = PausedPlayingRange;
            }
            PausedPlayingRange => {
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                self.state = PlayingRange;
                self.pipeline.as_mut().unwrap().play().await.unwrap();
                self.audio.play();
            }
            Stopped => self.select_media().await,
            PendingSelectMediaDecision => (),
        }
    }

    pub async fn seek(&mut self, target: Timestamp, flags: gst::SeekFlags) -> Result<(), ()> {
        use State::*;

        match self.state {
            Playing | EosPlaying => {
                match self.pipeline.as_mut().unwrap().seek(target, flags).await {
                    Ok(()) => {
                        if let EosPlaying = self.state {
                            self.state = Playing;
                        }
                    }
                    Err(SeekError::Eos) => {
                        // FIXME used to be an event
                        self.eos().await;
                    }
                    Err(SeekError::Unrecoverable) => {
                        // FIXME probably would need to report an error
                        self.stop();
                        return Err(());
                    }
                }
            }
            Paused | EosPaused => {
                match self.pipeline.as_mut().unwrap().seek(target, flags).await {
                    Ok(()) => {
                        if let EosPaused = self.state {
                            self.state = Paused;
                        }
                    }
                    Err(SeekError::Eos) => {
                        // FIXME used to be an event
                        self.eos().await;
                    }
                    Err(SeekError::Unrecoverable) => {
                        // FIXME probably would need to report an error
                        self.stop();
                        return Err(());
                    }
                }

                self.info.tick(target, self.state);
            }
            _ => (),
        }

        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut pipeline) = self.pipeline.take() {
            let _ = pipeline.stop();
            self.audio.dbl_renderer_impl = Some(pipeline.take_dbl_renderer_impl());
        }

        if let Some(abort_handle) = self.media_msg_abort_handle.take() {
            abort_handle.abort();
        }

        self.state = State::Stopped;
    }

    pub fn current_ts(&mut self) -> Option<Timestamp> {
        self.pipeline.as_mut().unwrap().current_ts()
    }

    pub fn redraw(&mut self) {
        self.audio.redraw();
    }

    pub async fn select_streams(&mut self, stream_ids: &[Arc<str>]) {
        let res = self
            .pipeline
            .as_mut()
            .unwrap()
            .select_streams(stream_ids)
            .await;

        match res {
            Ok(()) => self.streams_selected(),
            Err(SelectStreamsError::Unrecoverable) => self.stop(),
            Err(err) => panic!("{}", err),
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

    pub async fn play_range(&mut self, start: Timestamp) -> Result<(), ()> {
        match self.state {
            State::Paused | State::PlayingRange | State::PausedPlayingRange => {
                self.play_pause_btn.set_icon_name(Some(PAUSE_ICON));
                self.audio.start_play_range();
                self.state = State::PlayingRange;

                match self.pipeline.as_mut().unwrap().play_range(start).await {
                    Ok(()) => {
                        // FIXME probably need to reflect the state on the UI side
                    }
                    Err(SeekError::Eos) => {
                        // FIXME used to be an event
                        self.eos().await;
                    }
                    Err(SeekError::Unrecoverable) => {
                        // FIXME probably would need to report an error
                        self.stop();
                        return Err(());
                    }
                }
            }
            _ => (),
        }

        Ok(())
    }

    pub async fn play_range_done(&mut self) {
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

        if let State::PlayingRange = self.state {
            self.audio.stop_play_range();
            self.state = State::Paused;
        }
    }

    pub async fn eos(&mut self) {
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

        use State::*;
        match self.state {
            Paused => {
                self.audio.pause();
                self.state = EosPaused;
            }
            Playing => self.state = EosPlaying,
            PlayingRange => {
                self.audio.stop_play_range();
                self.state = EosPaused;
            }
            _ => (),
        }
    }

    pub async fn hold(&mut self) {
        main_panel::set_cursor_waiting();
        self.audio.pause();
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.pause().await.unwrap();
        };
    }

    pub async fn select_media(&mut self) {
        if let State::Playing | State::EosPlaying = self.state {
            self.hold().await;
        }

        self.state = State::PendingSelectMediaDecision;
        info_bar::hide();

        if let Some(ref last_path) = CONFIG.read().unwrap().media.last_path {
            self.file_dlg.set_current_folder(last_path);
        }
        self.file_dlg.show();
    }

    pub async fn open_media(&mut self, path: PathBuf) {
        self.cleanup();

        CONFIG.write().unwrap().media.last_path = path.parent().map(ToOwned::to_owned);
        info!(
            "{}",
            gettext("Opening {}...").replacen("{}", path.to_str().unwrap(), 1)
        );

        match pipeline::Playback::try_new(
            path.as_ref(),
            self.audio
                .dbl_renderer_impl
                .take()
                .expect("Couldn't take double visu renderer"),
            &self.video.video_sink(),
        )
        .await
        {
            Ok((pipeline, mut media_evt_rx)) => {
                if !pipeline.missing_plugins.is_empty() {
                    info_bar::show_info(gettext("Some streams are not usable. {}").replace(
                        "{}",
                        &Self::format_missing_plugins(&pipeline.missing_plugins),
                    ));
                }

                self.header_bar
                    .set_subtitle(Some(pipeline.info.read().unwrap().file_name.as_str()));

                self.audio.new_media(&pipeline);
                self.export.new_media(&pipeline);
                self.info.new_media(&pipeline);
                self.perspective.new_media(&pipeline);
                self.split.new_media(&pipeline);
                self.streams.new_media(&pipeline);
                self.video.new_media(&pipeline);

                // FIXME move the handler in a dedicated function (would need a playback::Controller)
                // FIXME we might want to merge it back with the UIEvent handler so that
                // MediaEvents are translated to UIEvents and posted immeditely
                let (media_evt_handler, abort_handle) = abortable(async move {
                    use MediaEvent::*;

                    while let Some(event) = media_evt_rx.next().await {
                        match event {
                            Eos => playback::eos(),
                            Error(err) => {
                                let err = gettext("An unrecoverable error occured. {}")
                                    .replace("{}", &err);
                                error!("{}", err);
                                info_bar::show_error(err);
                                break;
                            }
                            MustRefresh => audio::refresh(),
                            PlayRangeDone => playback::play_range_done(),
                            other => unreachable!("{:?}", other),
                        }
                    }
                });
                self.media_msg_abort_handle = Some(abort_handle);
                spawn(media_evt_handler.map(|_| ()));

                self.pipeline = Some(pipeline);

                self.streams_selected();

                self.audio.pause();
                self.state = State::Paused;
                main_panel::reset_cursor();
            }
            Err(error) => {
                main_panel::reset_cursor();

                use OpenError::*;
                let error = match error {
                    Generic(error) => error,
                    MissingPlugins(plugins) => Self::format_missing_plugins(&plugins),
                    StateChange => gettext("Failed to switch the media to Paused"),
                    GLSinkError => {
                        let mut config = CONFIG.write().expect("Failed to get CONFIG as mut");
                        config.media.is_gl_disabled = true;
                        config.save();

                        gettext(
        "Video rendering hardware acceleration seems broken and has been disabled.\nPlease restart the application.",
                        )
                    }
                };

                info_bar::show_error(gettext("Error opening file. {}").replace("{}", &error));
            }
        };
    }

    fn format_missing_plugins(plugins: &MissingPlugins) -> String {
        ngettext(
            "Missing plugin:\n{}",
            "Missing plugins:\n{}",
            plugins.len() as u32,
        )
        .replacen("{}", &format!("{}", plugins), 1)
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
    fn cleanup(&mut self) {
        self.stop();

        self.info.cleanup();
        self.audio.cleanup();
        self.video.cleanup();
        self.export.cleanup();
        self.split.cleanup();
        self.streams.cleanup();
        self.perspective.cleanup();
        self.header_bar.set_subtitle(Some(""));
    }
}
