use futures::channel::mpsc as async_mpsc;
use futures::prelude::*;
use futures::stream;

use gdk::{Cursor, CursorType, WindowExt};
use gettextrs::gettext;
use gio::prelude::*;
use glib::clone;
use gtk::prelude::*;

use log::{debug, trace};

use application::{CommandLineArguments, APP_PATH, CONFIG};
use media::MediaEvent;

use super::{
    spawn,
    ui_event::{self, UIEvent},
    AudioDispatcher, ExportDispatcher, InfoBarController, InfoDispatcher, MainController,
    PerspectiveDispatcher, PlaybackPipeline, SplitDispatcher, StreamsDispatcher, UIController,
    UIDispatcher, UIFocusContext, VideoDispatcher,
};

pub(super) struct MainDispatcher {
    app: gtk::Application,
    main_ctrl: MainController,
    window: gtk::ApplicationWindow,
    info_bar_ctrl: InfoBarController,
    saved_context: Option<UIFocusContext>,
    focus: UIFocusContext,
}

impl MainDispatcher {
    pub(super) fn setup(app: &gtk::Application, args: &CommandLineArguments) {
        let gst_init_res = gst::init();

        let builder = gtk::Builder::from_resource(&format!("{}/{}", *APP_PATH, "media-toc.ui"));

        let window: gtk::ApplicationWindow = builder.get_object("application-window").unwrap();
        window.set_application(Some(app));

        let (ui_event, ui_event_receiver) = ui_event::new_pair();
        let (media_event_sender, media_event_receiver) = async_mpsc::channel(1);

        let mut main_ctrl =
            MainController::new(&window, args, &builder, &ui_event, media_event_sender);
        main_ctrl.window_delete_id = Some(window.connect_delete_event(
            clone!(@strong ui_event => move |_, _| {
                ui_event.quit();
                Inhibit(true)
            }),
        ));

        let mut this = MainDispatcher {
            app: app.clone(),
            main_ctrl,
            window: window.clone(),
            info_bar_ctrl: InfoBarController::new(&app, &builder, &ui_event),
            saved_context: None,
            focus: UIFocusContext::PlaybackPage,
        };

        let app_menu = gio::Menu::new();
        app.set_app_menu(Some(&app_menu));

        let app_section = gio::Menu::new();
        app_menu.append_section(None, &app_section);

        // About
        let about = gio::SimpleAction::new("about", None);
        app.add_action(&about);
        about.connect_activate(clone!(@strong ui_event => move |_, _| {
            ui_event.about();
        }));
        app.set_accels_for_action("app.about", &["<Ctrl>A"]);
        app_section.append(Some(&gettext("About")), Some("app.about"));

        // Quit
        let quit = gio::SimpleAction::new("quit", None);
        app.add_action(&quit);
        quit.connect_activate(clone!(@strong ui_event => move |_, _| {
            ui_event.quit();
        }));
        app.set_accels_for_action("app.quit", &["<Ctrl>Q"]);
        app_section.append(Some(&gettext("Quit")), Some("app.quit"));

        if gst_init_res.is_ok() {
            let _ = PlaybackPipeline::check_requirements()
                .map_err(clone!(@strong ui_event => move |err| ui_event.show_error(err)));

            let main_section = gio::Menu::new();
            app_menu.insert_section(0, None, &main_section);

            // Register Open action
            let open = gio::SimpleAction::new("open", None);
            app.add_action(&open);
            open.connect_activate(clone!(@strong ui_event => move |_, _| {
                ui_event.select_media();
            }));
            main_section.append(Some(&gettext("Open media file")), Some("app.open"));
            app.set_accels_for_action("app.open", &["<Ctrl>O"]);

            // Register Play/Pause action
            let play_pause = gio::SimpleAction::new("play_pause", None);
            app.add_action(&play_pause);
            play_pause.connect_activate(clone!(@strong ui_event => move |_, _| {
                ui_event.play_pause();
            }));
            this.main_ctrl.play_pause_btn.set_sensitive(true);

            let display_page: gtk::Box = builder.get_object("display-box").unwrap();
            display_page.connect_map(clone!(@strong ui_event => move |_| {
                ui_event.switch_to(UIFocusContext::PlaybackPage);
            }));

            PerspectiveDispatcher::setup(&mut this.main_ctrl.perspective_ctrl, &app, &ui_event);
            VideoDispatcher::setup(&mut this.main_ctrl.video_ctrl, &app, &ui_event);
            InfoDispatcher::setup(&mut this.main_ctrl.info_ctrl, &app, &ui_event);
            AudioDispatcher::setup(&mut this.main_ctrl.audio_ctrl, &app, &ui_event);
            ExportDispatcher::setup(&mut this.main_ctrl.export_ctrl, &app, &ui_event);
            SplitDispatcher::setup(&mut this.main_ctrl.split_ctrl, &app, &ui_event);
            StreamsDispatcher::setup(&mut this.main_ctrl.streams_ctrl, &app, &ui_event);

            ui_event.switch_to(UIFocusContext::PlaybackPage);

            {
                let config = CONFIG.read().unwrap();
                if config.ui.width > 0 && config.ui.height > 0 {
                    window.resize(config.ui.width, config.ui.height);
                    this.main_ctrl
                        .playback_paned
                        .set_position(config.ui.paned_pos);
                }
            }

            let open_btn: gtk::Button = builder.get_object("open-btn").unwrap();
            open_btn.set_sensitive(true);

            Self::spawn_event_handlers(this, ui_event_receiver, media_event_receiver);
            ui_event.show_all();

            if let Some(input_file) = args.input_file.to_owned() {
                ui_event.open_media(input_file);
            }
        } else {
            // GStreamer initialization failed
            Self::spawn_event_handlers(this, ui_event_receiver, media_event_receiver);
            ui_event.show_all();

            let msg = gettext("Failed to initialize GStreamer, the application can't be used.");
            ui_event.show_error(msg);
        }
    }
}

impl MainDispatcher {
    fn spawn_event_handlers(
        mut self,
        ui_event_receiver: async_mpsc::UnboundedReceiver<UIEvent>,
        media_event_receiver: async_mpsc::Receiver<MediaEvent>,
    ) {
        enum Item {
            UI(UIEvent),
            Media(MediaEvent),
        }

        spawn(async move {
            let mut combined_streams = stream::select(
                ui_event_receiver.map(Item::UI),
                media_event_receiver.map(Item::Media),
            );

            while let Some(event) = combined_streams.next().await {
                match event {
                    Item::UI(event) => {
                        if let UIEvent::Tick = event {
                            trace!("handling event {:?}", event);
                        } else {
                            debug!("handling event {:?}", event);
                        }
                        if self.handle(event).await.is_err() {
                            break;
                        }
                    }
                    Item::Media(event) => self.main_ctrl.handle_media_event(event),
                }
            }
        });
    }

    async fn handle(&mut self, event: UIEvent) -> Result<(), ()> {
        use UIEvent::*;

        match event {
            About => self.main_ctrl.about(),
            ActionOver(focus_ctx) => match focus_ctx {
                UIFocusContext::ExportPage => self.main_ctrl.export_ctrl.switch_to_available(),
                UIFocusContext::SplitPage => self.main_ctrl.split_ctrl.switch_to_available(),
                _ => unreachable!(),
            },
            AddChapter => {
                self.main_ctrl.add_chapter();
                self.update_focus(self.focus);
            }
            AskQuestion {
                question,
                response_sender,
            } => self.info_bar_ctrl.ask_question(&question, response_sender),
            AudioAreaEvent(event) => self.main_ctrl.audio_area_event(event),
            CancelSelectMedia => self.main_ctrl.cancel_select_media(),
            ChapterClicked(chapter_path) => self.main_ctrl.chapter_clicked(chapter_path),
            HideInfoBar => self.info_bar_ctrl.hide(),
            NextChapter => self.main_ctrl.next_chapter(),
            OpenMedia(path) => self.main_ctrl.open_media(path),
            PlayPause => self.main_ctrl.play_pause(),
            PlayRange {
                start,
                end,
                ts_to_restore,
            } => {
                self.main_ctrl.play_range(start, end, ts_to_restore);
            }
            PreviousChapter => self.main_ctrl.previous_chapter(),
            Quit => {
                self.main_ctrl.quit();
                return Err(());
            }
            RefreshInfo(ts) => self.main_ctrl.refresh_info(ts),
            RemoveChapter => {
                self.main_ctrl.remove_chapter();
                self.update_focus(self.focus);
            }
            RenameChapter(new_title) => {
                self.main_ctrl.rename_chapter(&new_title);
                self.restore_context();
            }
            ResetCursor => self.reset_cursor(),
            RestoreContext => self.restore_context(),
            Seek { target, flags } => self.main_ctrl.seek(target, flags),
            SelectMedia => self.main_ctrl.select_media(),
            SetCursorDoubleArrow => self.set_cursor_double_arrow(),
            SetCursorWaiting => self.set_cursor_waiting(),
            ShowAll => self.show_all(),
            ShowError(msg) => self.info_bar_ctrl.show_error(&msg),
            ShowInfo(msg) => self.info_bar_ctrl.show_info(&msg),
            StepBack => self.main_ctrl.step_back(),
            StepForward => self.main_ctrl.step_forward(),
            StreamClicked(type_) => self.main_ctrl.stream_clicked(type_),
            StreamExportToggled(type_, tree_path) => {
                self.main_ctrl.stream_export_toggled(type_, tree_path)
            }
            SwitchTo(focus_ctx) => self.switch_to(focus_ctx),
            TemporarilySwitchTo(focus_ctx) => {
                self.save_context();
                self.bind_accels_for(focus_ctx);
            }
            Tick => self.main_ctrl.audio_ctrl.tick(),
            ToggleChapterList(must_show) => self.main_ctrl.info_ctrl.toggle_chapter_list(must_show),
            ToggleRepeat(must_repeat) => self.main_ctrl.info_ctrl.repeat_chapter = must_repeat,
            TriggerAction(focus_ctx) => match focus_ctx {
                UIFocusContext::ExportPage => {
                    self.main_ctrl
                        .trigger_output_action::<ExportDispatcher>()
                        .await
                }
                UIFocusContext::SplitPage => {
                    self.main_ctrl
                        .trigger_output_action::<SplitDispatcher>()
                        .await
                }
                _ => unreachable!(),
            },
            UpdateAudioRenderingCndt { dimensions } => {
                self.main_ctrl.audio_ctrl.update_conditions(dimensions)
            }
            UpdateFocus => self.update_focus(self.focus),
            ZoomIn => self.main_ctrl.audio_ctrl.zoom_in(),
            ZoomOut => self.main_ctrl.audio_ctrl.zoom_out(),
        }

        Ok(())
    }

    pub fn show_all(&self) {
        self.window.show();
        self.window.activate();
    }

    fn set_cursor_waiting(&self) {
        if let Some(gdk_window) = self.window.get_window() {
            gdk_window.set_cursor(Some(&Cursor::new_for_display(
                &gdk_window.get_display(),
                CursorType::Watch,
            )));
        }
    }

    fn set_cursor_double_arrow(&self) {
        if let Some(gdk_window) = self.window.get_window() {
            gdk_window.set_cursor(Some(&Cursor::new_for_display(
                &gdk_window.get_display(),
                CursorType::SbHDoubleArrow,
            )));
        }
    }

    fn reset_cursor(&self) {
        if let Some(gdk_window) = self.window.get_window() {
            gdk_window.set_cursor(None);
        }
    }

    fn bind_accels_for(&self, ctx: UIFocusContext) {
        match ctx {
            UIFocusContext::PlaybackPage => {
                self.app
                    .set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);
                self.app
                    .set_accels_for_action("app.next_chapter", &["Down", "AudioNext"]);
                self.app
                    .set_accels_for_action("app.previous_chapter", &["Up", "AudioPrev"]);
                self.app.set_accels_for_action("app.close_info_bar", &[]);
            }
            UIFocusContext::ExportPage
            | UIFocusContext::SplitPage
            | UIFocusContext::StreamsPage => {
                self.app
                    .set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);
                self.app
                    .set_accels_for_action("app.next_chapter", &["AudioNext"]);
                self.app
                    .set_accels_for_action("app.previous_chapter", &["AudioPrev"]);
                self.app.set_accels_for_action("app.close_info_bar", &[]);
            }
            UIFocusContext::TextEntry => {
                self.app
                    .set_accels_for_action("app.play_pause", &["AudioPlay"]);
                self.app.set_accels_for_action("app.next_chapter", &[]);
                self.app.set_accels_for_action("app.previous_chapter", &[]);
                self.app.set_accels_for_action("app.close_info_bar", &[]);
            }
            UIFocusContext::InfoBar => {
                self.app
                    .set_accels_for_action("app.play_pause", &["AudioPlay"]);
                self.app.set_accels_for_action("app.next_chapter", &[]);
                self.app.set_accels_for_action("app.previous_chapter", &[]);
                self.app
                    .set_accels_for_action("app.close_info_bar", &["Escape"]);
            }
        }

        PerspectiveDispatcher::bind_accels_for(ctx, &self.app);
        VideoDispatcher::bind_accels_for(ctx, &self.app);
        InfoDispatcher::bind_accels_for(ctx, &self.app);
        AudioDispatcher::bind_accels_for(ctx, &self.app);
        ExportDispatcher::bind_accels_for(ctx, &self.app);
        SplitDispatcher::bind_accels_for(ctx, &self.app);
        StreamsDispatcher::bind_accels_for(ctx, &self.app);
    }

    fn update_focus(&mut self, ctx: UIFocusContext) {
        if self.focus == UIFocusContext::PlaybackPage && ctx != UIFocusContext::PlaybackPage {
            self.main_ctrl.info_ctrl.loose_focus();
        }

        match ctx {
            UIFocusContext::ExportPage => self.main_ctrl.export_ctrl.grab_focus(),
            UIFocusContext::PlaybackPage => self.main_ctrl.info_ctrl.grab_focus(),
            UIFocusContext::SplitPage => self.main_ctrl.split_ctrl.grab_focus(),
            UIFocusContext::StreamsPage => self.main_ctrl.streams_ctrl.grab_focus(),
            _ => (),
        }

        self.focus = ctx;
    }

    fn switch_to(&mut self, ctx: UIFocusContext) {
        self.bind_accels_for(ctx);
        self.update_focus(ctx);
    }

    fn save_context(&mut self) {
        self.saved_context = Some(self.focus);
    }

    fn restore_context(&mut self) {
        if let Some(focus_ctx) = self.saved_context.take() {
            self.switch_to(focus_ctx);
        }
    }
}
