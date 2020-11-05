use futures::channel::mpsc as async_mpsc;
use futures::prelude::*;

use gdk::{Cursor, CursorType, WindowExt};
use gettextrs::gettext;
use gio::prelude::*;
use glib::clone;
use gtk::prelude::*;

use log::{debug, trace};

use std::{cell::RefCell, rc::Rc};

use media::MediaEvent;

use super::{
    main_controller::ControllerState, spawn, ui_event::UIEvent, AudioDispatcher, ExportDispatcher,
    InfoBarController, InfoDispatcher, MainController, PerspectiveDispatcher, PlaybackPipeline,
    SplitDispatcher, StreamsDispatcher, UIController, UIDispatcher, UIFocusContext,
    VideoDispatcher,
};

pub(super) struct MainDispatcher {
    app: gtk::Application,
    window: gtk::ApplicationWindow,
    main_ctrl: Rc<RefCell<MainController>>,
    info_bar_ctrl: InfoBarController,
    saved_context: Option<UIFocusContext>,
    focus: UIFocusContext,
}

impl MainDispatcher {
    pub(super) fn setup(
        main_ctrl: &mut MainController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        app: &gtk::Application,
        window: &gtk::ApplicationWindow,
        builder: &gtk::Builder,
        mut ui_event_receiver: async_mpsc::UnboundedReceiver<UIEvent>,
    ) {
        let mut handler = MainDispatcher {
            app: app.clone(),
            window: window.clone(),
            main_ctrl: Rc::clone(&main_ctrl_rc),
            info_bar_ctrl: InfoBarController::new(app, builder, main_ctrl.ui_event()),
            saved_context: None,
            focus: UIFocusContext::PlaybackPage,
        };

        spawn(async move {
            while let Some(event) = ui_event_receiver.next().await {
                if let UIEvent::Tick = event {
                    trace!("handling event {:?}", event);
                } else {
                    debug!("handling event {:?}", event);
                }
                if handler.handle(event).await.is_err() {
                    break;
                }
            }
        });

        let app_menu = gio::Menu::new();
        app.set_app_menu(Some(&app_menu));

        let app_section = gio::Menu::new();
        app_menu.append_section(None, &app_section);

        // About
        let about = gio::SimpleAction::new("about", None);
        app.add_action(&about);
        about.connect_activate(
            clone!(@strong main_ctrl.ui_event as ui_event => move |_, _| {
                ui_event.about();
            }),
        );
        app.set_accels_for_action("app.about", &["<Ctrl>A"]);
        app_section.append(Some(&gettext("About")), Some("app.about"));

        // Quit
        let quit = gio::SimpleAction::new("quit", None);
        app.add_action(&quit);
        quit.connect_activate(
            clone!(@strong main_ctrl.ui_event as ui_event => move |_, _| {
                ui_event.quit();
            }),
        );
        app.set_accels_for_action("app.quit", &["<Ctrl>Q"]);
        app_section.append(Some(&gettext("Quit")), Some("app.quit"));

        main_ctrl.window_delete_id = Some(main_ctrl.window.connect_delete_event(
            clone!(@strong main_ctrl.ui_event as ui_event => move |_, _| {
                ui_event.quit();
                Inhibit(true)
            }),
        ));

        let ui_event = main_ctrl.ui_event().clone();
        if gst::init().is_ok() {
            main_ctrl.new_media_event_handler = Some(Box::new(
                clone!(@weak main_ctrl_rc => @default-panic, move |receiver| {
                    let main_ctrl_rc = Rc::clone(&main_ctrl_rc);
                    async move {
                        let mut receiver = receiver;
                        while let Some(event) =
                            async_mpsc::Receiver::<MediaEvent>::next(&mut receiver).await
                        {
                            if main_ctrl_rc.borrow_mut().handle_media_event(event).is_err() {
                                break;
                            }
                        }
                        debug!("Media event handler terminated");
                    }.boxed_local()
                }),
            ));

            let ui_event_clone = ui_event.clone();
            let _ = PlaybackPipeline::check_requirements()
                .map_err(move |err| ui_event_clone.show_error(err));

            let main_section = gio::Menu::new();
            app_menu.insert_section(0, None, &main_section);

            // Register Open action
            let open = gio::SimpleAction::new("open", None);
            app.add_action(&open);
            open.connect_activate(clone!(@weak main_ctrl_rc => move |_, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                match main_ctrl.state {
                    ControllerState::Playing | ControllerState::EOS => {
                        main_ctrl.hold();
                        main_ctrl.state = ControllerState::PendingSelectMedia;
                    }
                    _ => main_ctrl.select_media(),
                }
            }));
            main_section.append(Some(&gettext("Open media file")), Some("app.open"));
            app.set_accels_for_action("app.open", &["<Ctrl>O"]);

            main_ctrl.open_btn.set_sensitive(true);

            // Register Play/Pause action
            let play_pause = gio::SimpleAction::new("play_pause", None);
            app.add_action(&play_pause);
            play_pause.connect_activate(clone!(@weak main_ctrl_rc => move |_, _| {
                main_ctrl_rc.borrow_mut().play_pause();
            }));
            main_ctrl.play_pause_btn.set_sensitive(true);

            let ui_event_clone = ui_event.clone();
            main_ctrl.display_page.connect_map(move |_| {
                ui_event_clone.switch_to(UIFocusContext::PlaybackPage);
            });

            PerspectiveDispatcher::setup(
                &mut main_ctrl.perspective_ctrl,
                main_ctrl_rc,
                &app,
                &ui_event,
            );
            VideoDispatcher::setup(&mut main_ctrl.video_ctrl, main_ctrl_rc, &app, &ui_event);
            InfoDispatcher::setup(&mut main_ctrl.info_ctrl, main_ctrl_rc, &app, &ui_event);
            AudioDispatcher::setup(&mut main_ctrl.audio_ctrl, main_ctrl_rc, &app, &ui_event);
            ExportDispatcher::setup(&mut main_ctrl.export_ctrl, main_ctrl_rc, &app, &ui_event);
            SplitDispatcher::setup(&mut main_ctrl.split_ctrl, main_ctrl_rc, &app, &ui_event);
            StreamsDispatcher::setup(&mut main_ctrl.streams_ctrl, main_ctrl_rc, &app, &ui_event);

            ui_event.switch_to(UIFocusContext::PlaybackPage);
        } else {
            // GStreamer initialization failed
            let msg = gettext("Failed to initialize GStreamer, the application can't be used.");
            ui_event.show_error(msg);
        }
    }
}

impl MainDispatcher {
    async fn handle(&mut self, event: UIEvent) -> Result<(), ()> {
        use UIEvent::*;

        match event {
            About => self.main_ctrl.borrow().about(),
            AskQuestion {
                question,
                response_sender,
            } => self.info_bar_ctrl.ask_question(&question, response_sender),
            AudioAreaEvent(event) => self.main_ctrl.borrow_mut().audio_area_event(event),
            CancelSelectMedia => self.main_ctrl.borrow_mut().cancel_select_media(),
            ChapterClicked(chapter_path) => {
                self.main_ctrl.borrow_mut().chapter_clicked(chapter_path)
            }
            HideInfoBar => self.info_bar_ctrl.hide(),
            OpenMedia(path) => self.main_ctrl.borrow_mut().open_media(path),
            NextChapter => self.main_ctrl.borrow_mut().next_chapter(),
            PlayRange {
                start,
                end,
                ts_to_restore,
            } => {
                self.main_ctrl
                    .borrow_mut()
                    .play_range(start, end, ts_to_restore);
            }
            PreviousChapter => self.main_ctrl.borrow_mut().previous_chapter(),
            Quit => {
                self.main_ctrl.borrow_mut().quit();
                return Err(());
            }
            RefreshInfo(ts) => self.main_ctrl.borrow_mut().refresh_info(ts),
            ResetCursor => self.reset_cursor(),
            RestoreContext => self.restore_context(),
            Seek { target, flags } => self.main_ctrl.borrow_mut().seek(target, flags),
            SetCursorDoubleArrow => self.set_cursor_double_arrow(),
            SetCursorWaiting => self.set_cursor_waiting(),
            ShowAll => self.show_all(),
            ShowError(msg) => self.info_bar_ctrl.show_error(&msg),
            ShowInfo(msg) => self.info_bar_ctrl.show_info(&msg),
            StepBack => self.main_ctrl.borrow_mut().step_back(),
            StepForward => self.main_ctrl.borrow_mut().step_forward(),
            StreamClicked(type_) => self.main_ctrl.borrow_mut().stream_clicked(type_),
            StreamExportToggled(type_, tree_path) => self
                .main_ctrl
                .borrow_mut()
                .stream_export_toggled(type_, tree_path),
            SwitchTo(focus_ctx) => self.switch_to(focus_ctx),
            TemporarilySwitchTo(focus_ctx) => {
                self.save_context();
                self.bind_accels_for(focus_ctx);
            }
            Tick => self.main_ctrl.borrow_mut().audio_ctrl.tick(),
            ToggleChapterList(must_show) => self
                .main_ctrl
                .borrow()
                .info_ctrl
                .toggle_chapter_list(must_show),
            ToggleRepeat(must_repeat) => {
                self.main_ctrl.borrow_mut().info_ctrl.repeat_chapter = must_repeat
            }
            UpdateAudioRenderingCndt { dimensions } => self
                .main_ctrl
                .borrow_mut()
                .audio_ctrl
                .update_conditions(dimensions),
            UpdateFocus => self.update_focus(self.focus),
            ZoomIn => self.main_ctrl.borrow_mut().audio_ctrl.zoom_in(),
            ZoomOut => self.main_ctrl.borrow_mut().audio_ctrl.zoom_out(),
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
        let main_ctrl = self.main_ctrl.borrow();

        if self.focus == UIFocusContext::PlaybackPage && ctx != UIFocusContext::PlaybackPage {
            main_ctrl.info_ctrl.loose_focus();
        }

        match ctx {
            UIFocusContext::ExportPage => main_ctrl.export_ctrl.grab_focus(),
            UIFocusContext::PlaybackPage => main_ctrl.info_ctrl.grab_focus(),
            UIFocusContext::SplitPage => main_ctrl.split_ctrl.grab_focus(),
            UIFocusContext::StreamsPage => main_ctrl.streams_ctrl.grab_focus(),
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
