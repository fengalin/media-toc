use gdk::{Cursor, CursorType, WindowExt};
use gettextrs::{gettext, ngettext};

use glib;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use log::{debug, error, info};

use std::{cell::RefCell, collections::HashSet, path::Path, rc::Rc, sync::Arc};

use crate::{
    application::{APP_ID, APP_PATH, CONFIG},
    media::{MediaEvent, PlaybackPipeline, PlaybackState},
};

use super::{
    AudioController, ChaptersBoundaries, ExportController, InfoController, PerspectiveController,
    PositionStatus, SplitController, StreamsController, UIController, VideoController,
};

const PAUSE_ICON: &str = "media-playback-pause-symbolic";
const PLAYBACK_ICON: &str = "media-playback-start-symbolic";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControllerState {
    EOS,
    Paused,
    PendingTakePipeline,
    PendingSelectMedia,
    Playing,
    PlayingRange(u64),
    Seeking,
    Stopped,
    TwoStepsSeek(u64),
}

pub struct MainController {
    pub(super) window: gtk::ApplicationWindow,

    header_bar: gtk::HeaderBar,
    pub(super) open_btn: gtk::Button,
    playback_paned: gtk::Paned,
    pub(super) play_pause_btn: gtk::ToolButton,
    pub(super) info_bar_revealer: gtk::Revealer,
    pub(super) info_bar: gtk::InfoBar,
    info_bar_lbl: gtk::Label,
    info_bar_btn_box: gtk::ButtonBox,

    pub(super) perspective_ctrl: PerspectiveController,
    pub(super) video_ctrl: VideoController,
    pub(super) info_ctrl: InfoController,
    pub(super) audio_ctrl: AudioController,
    pub(super) export_ctrl: ExportController,
    pub(super) split_ctrl: SplitController,
    pub(super) streams_ctrl: StreamsController,

    pub(super) pipeline: Option<PlaybackPipeline>,
    take_pipeline_cb: Option<Box<dyn FnMut(&mut MainController, PlaybackPipeline)>>,
    missing_plugins: HashSet<String>,
    pub(super) state: ControllerState,

    info_bar_response_src: Option<glib::signal::SignalHandlerId>,
    pub(super) select_media_fn: Option<Rc<Fn()>>,
    pub(super) media_event_aync_handler: Option<Rc<Fn(MediaEvent) -> glib::Continue>>,
    media_event_aync_handler_src: Option<glib::SourceId>,
}

impl MainController {
    pub fn new_rc(disable_gl: bool) -> Rc<RefCell<Self>> {
        let builder = gtk::Builder::new_from_resource(&format!("{}/{}", *APP_PATH, "media-toc.ui"));
        let chapters_boundaries = Rc::new(RefCell::new(ChaptersBoundaries::new()));

        Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").unwrap(),
            header_bar: builder.get_object("header-bar").unwrap(),
            open_btn: builder.get_object("open-btn").unwrap(),
            playback_paned: builder.get_object("playback-paned").unwrap(),
            play_pause_btn: builder.get_object("play_pause-toolbutton").unwrap(),
            info_bar_revealer: builder.get_object("info_bar-revealer").unwrap(),
            info_bar: builder.get_object("info_bar").unwrap(),
            info_bar_lbl: builder.get_object("info_bar-lbl").unwrap(),
            info_bar_btn_box: builder.get_object("info_bar-btnbox").unwrap(),

            perspective_ctrl: PerspectiveController::new(&builder),
            video_ctrl: VideoController::new(&builder, disable_gl),
            info_ctrl: InfoController::new(&builder, Rc::clone(&chapters_boundaries)),
            audio_ctrl: AudioController::new(&builder, chapters_boundaries),
            export_ctrl: ExportController::new(&builder),
            split_ctrl: SplitController::new(&builder),
            streams_ctrl: StreamsController::new(&builder),

            pipeline: None,
            take_pipeline_cb: None,
            missing_plugins: HashSet::<String>::new(),
            state: ControllerState::Stopped,

            info_bar_response_src: None,
            select_media_fn: None,
            media_event_aync_handler: None,
            media_event_aync_handler_src: None,
        }))
    }

    pub fn setup(&mut self, is_gst_ok: bool) {
        self.info_bar
            .add_button(&gettext("Yes"), gtk::ResponseType::Yes);
        self.info_bar
            .add_button(&gettext("No"), gtk::ResponseType::No);
        self.info_bar
            .add_button(&gettext("Cancel"), gtk::ResponseType::Cancel);

        if is_gst_ok {
            {
                let config = CONFIG.read().unwrap();
                if config.ui.width > 0 && config.ui.height > 0 {
                    self.window.resize(config.ui.width, config.ui.height);
                    self.playback_paned.set_position(config.ui.paned_pos);
                }

                self.open_btn.set_sensitive(true);
            }

            self.perspective_ctrl.setup();
            self.video_ctrl.setup();
            self.info_ctrl.setup();
            self.audio_ctrl.setup();
            self.export_ctrl.setup();
            self.split_ctrl.setup();
            self.streams_ctrl.setup();
        }
    }

    pub fn show_all(&self) {
        self.window.show();
        self.window.activate();
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
        dialog.set_copyright(Some(&gettext("© 2017–2019 François Laignel")));
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
        self.remove_media_event_aync_handler();

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

    pub fn show_message<Msg: AsRef<str>>(&mut self, type_: gtk::MessageType, message: Msg) {
        if type_ == gtk::MessageType::Question {
            self.info_bar_btn_box.set_visible(true);
            self.info_bar.set_show_close_button(false);
        } else {
            if let Some(src) = self.info_bar_response_src.take() {
                self.info_bar.disconnect(src);
            }
            self.info_bar_btn_box.set_visible(false);
            self.info_bar.set_show_close_button(true);
        }

        self.info_bar.set_message_type(type_);
        self.info_bar_lbl.set_label(message.as_ref());
        self.info_bar_revealer.set_reveal_child(true);
    }

    pub fn show_error<Msg: AsRef<str>>(&mut self, message: Msg) {
        error!("{}", message.as_ref());
        self.show_message(gtk::MessageType::Question, message);
    }

    pub fn show_info<Msg: AsRef<str>>(&mut self, message: Msg) {
        info!("{}", message.as_ref());
        self.show_message(gtk::MessageType::Info, message);
    }

    pub fn show_question<Q: AsRef<str>>(
        &mut self,
        question: Q,
        response_cb: Box<Fn(gtk::ResponseType)>,
    ) {
        let info_bar_revealer = self.info_bar_revealer.clone();
        if let Some(src) = self.info_bar_response_src.take() {
            self.info_bar.disconnect(src);
        }
        self.info_bar_response_src =
            Some(self.info_bar.connect_response(move |_, response_type| {
                info_bar_revealer.set_reveal_child(false);
                response_cb(response_type);
            }));
        self.show_message(gtk::MessageType::Question, question);
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
            match pipeline.get_state() {
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
            self.seek(0, gst::SeekFlags::ACCURATE);
        }
    }

    pub fn move_chapter_boundary(&mut self, boundary: u64, to_position: u64) -> PositionStatus {
        self.info_ctrl.move_chapter_boundary(boundary, to_position)
    }

    pub fn seek(&mut self, position: u64, flags: gst::SeekFlags) {
        let mut seek_pos = position;
        let mut flags = flags;
        self.state = match self.state {
            ControllerState::Playing => ControllerState::Seeking,
            ControllerState::Paused => {
                flags = gst::SeekFlags::ACCURATE;
                let seek_1st_step = self.audio_ctrl.get_seek_back_1st_position(position);
                match seek_1st_step {
                    Some(seek_1st_step) => {
                        seek_pos = seek_1st_step;
                        ControllerState::TwoStepsSeek(position)
                    }
                    None => ControllerState::Seeking,
                }
            }
            ControllerState::TwoStepsSeek(target) => {
                // seeked position and target might be different if the user
                // seeks repeatedly and rapidly: we can receive a new seek while still
                // being in the `TwoStepsSeek` step from previous seek.
                // Currently, I think it is better to favor completing the in-progress
                // `TwoStepsSeek` (which purpose is to center the cursor on the waveform)
                // than reaching for the latest seeked position
                seek_pos = target;
                ControllerState::Seeking
            }
            ControllerState::EOS => {
                self.audio_ctrl.switch_to_playing();
                ControllerState::Seeking
            }
            _ => return,
        };

        self.info_ctrl.seek(seek_pos, &self.state);
        self.audio_ctrl.seek(seek_pos);
        self.pipeline.as_ref().unwrap().seek(seek_pos, flags);
    }

    pub fn play_range(&mut self, start: u64, end: u64, pos_to_restore: u64) {
        if self.state == ControllerState::Paused {
            self.info_ctrl.start_play_range();
            self.audio_ctrl.start_play_range();

            self.state = ControllerState::PlayingRange(pos_to_restore);
            self.pipeline.as_ref().unwrap().seek_range(start, end);
        }
    }

    pub fn get_position(&mut self) -> u64 {
        self.pipeline.as_mut().unwrap().get_position()
    }

    pub fn refresh(&mut self) {
        self.audio_ctrl.redraw();
    }

    pub fn refresh_info(&mut self, position: u64) {
        match self.state {
            ControllerState::Seeking => (),
            _ => self.info_ctrl.tick(position, false),
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
        self.switch_to_busy();
        self.audio_ctrl.switch_to_not_playing();
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

        if let Some(pipeline) = self.pipeline.as_mut() {
            pipeline.pause().unwrap();
        };
    }

    pub fn request_pipeline(
        &mut self,
        callback: Box<dyn FnMut(&mut MainController, PlaybackPipeline)>,
    ) {
        self.audio_ctrl.switch_to_not_playing();
        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));

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
            self.info_ctrl.export_chapters(&mut pipeline);
            let mut callback = self.take_pipeline_cb.take().unwrap();
            callback(self, pipeline);
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

    #[allow(clippy::redundant_closure)]
    fn attach_media_event_async_handler(&mut self, receiver: glib::Receiver<MediaEvent>) {
        let media_event_aync_handler = Rc::clone(
            self.media_event_aync_handler
                .as_ref()
                .expect("MainController: media_event_async handler is not defined"),
        );

        self.media_event_aync_handler_src =
            Some(receiver.attach(None, move |event| media_event_aync_handler(event)));
    }

    fn remove_media_event_aync_handler(&mut self) {
        if let Some(source_id) = self.media_event_aync_handler_src.take() {
            glib::source_remove(source_id);
        }
    }

    pub fn handle_media_event(&mut self, event: MediaEvent) -> glib::Continue {
        let mut keep_going = true;

        match event {
            MediaEvent::AsyncDone(playback_state) => {
                if let ControllerState::Seeking = self.state {
                    self.state = match playback_state {
                        PlaybackState::Playing => ControllerState::Playing,
                        PlaybackState::Paused => ControllerState::Paused,
                    };
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
                    self.show_error(message);
                }

                self.audio_ctrl.switch_to_not_playing();
                self.state = ControllerState::Paused;
            }
            MediaEvent::MissingPlugin(plugin) => {
                error!(
                    "{}",
                    gettext("Missing plugin: {}").replacen("{}", &plugin, 1)
                );
                self.missing_plugins.insert(plugin);
            }
            MediaEvent::ReadyForRefresh => match self.state {
                ControllerState::Playing => (),
                ControllerState::Paused => {
                    self.refresh();
                }
                ControllerState::TwoStepsSeek(target) => {
                    self.seek(target, gst::SeekFlags::ACCURATE);
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
                        self.audio_ctrl.stop_play_range();
                        self.seek(pos_to_restore, gst::SeekFlags::ACCURATE);
                    }
                    _ => {
                        self.play_pause_btn.set_icon_name(Some(PLAYBACK_ICON));
                        self.audio_ctrl.switch_to_not_playing();
                        self.state = ControllerState::EOS;
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
            self.remove_media_event_aync_handler();
            self.audio_ctrl.switch_to_not_playing();
        }

        glib::Continue(keep_going)
    }

    pub fn set_cursor_waiting(&self) {
        let gdk_window = self.window.get_window().unwrap();
        gdk_window.set_cursor(Some(&Cursor::new_for_display(
            &gdk_window.get_display(),
            CursorType::Watch,
        )));
    }

    pub fn reset_cursor(&self) {
        self.window.get_window().unwrap().set_cursor(None);
    }

    pub fn switch_to_busy(&mut self) {
        self.set_cursor_waiting();
    }

    pub fn switch_to_default(&mut self) {
        self.reset_cursor();
    }

    pub fn select_media(&mut self) {
        let select_media_fn = Rc::clone(
            self.select_media_fn
                .as_ref()
                .expect("MainController: select_media_fn is not defined"),
        );
        gtk::idle_add(move || {
            select_media_fn();
            glib::Continue(false)
        });
    }

    pub fn open_media(&mut self, filepath: &Path) {
        if let Some(ref pipeline) = self.pipeline.take() {
            pipeline.stop();
        }

        self.remove_media_event_aync_handler();

        self.info_ctrl.cleanup();
        self.audio_ctrl.cleanup();
        self.video_ctrl.cleanup();
        self.export_ctrl.cleanup();
        self.split_ctrl.cleanup();
        self.streams_ctrl.cleanup();
        self.perspective_ctrl.cleanup();
        self.header_bar.set_subtitle(Some(""));

        let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);

        self.state = ControllerState::Stopped;
        self.missing_plugins.clear();
        self.attach_media_event_async_handler(receiver);

        let dbl_buffer_mtx = Arc::clone(&self.audio_ctrl.dbl_buffer_mtx);
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
