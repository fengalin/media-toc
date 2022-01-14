use futures::prelude::*;

use gdk::{Cursor, CursorType};
use gettextrs::gettext;
use gio::prelude::*;
use gtk::prelude::*;

use application::{CommandLineArguments, APP_PATH, CONFIG};
use media::PlaybackPipeline;

use crate::{
    audio, export, info, info_bar, main, perspective, playback, prelude::*, spawn, split, streams,
    video, UIEvent,
};

pub struct Dispatcher {
    app: gtk::Application,
    main_ctrl: main::Controller,
    window: gtk::ApplicationWindow,
    saved_context: Option<UIFocusContext>,
    focus: UIFocusContext,
}

impl Dispatcher {
    pub fn setup(app: &gtk::Application, args: &CommandLineArguments) {
        let gst_init_res = gst::init();

        let builder = gtk::Builder::from_resource(&format!("{}/{}", *APP_PATH, "media-toc.ui"));

        let window: gtk::ApplicationWindow = builder.get_object("application-window").unwrap();
        window.set_application(Some(app));

        let mut main_ctrl = main::Controller::new(&window, args, &builder);
        main_ctrl.window_delete_id = Some(window.connect_delete_event(|_, _| {
            main::quit();
            Inhibit(true)
        }));

        let mut this = Dispatcher {
            app: app.clone(),
            main_ctrl,
            window: window.clone(),
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
        about.connect_activate(|_, _| main::about());
        app.set_accels_for_action("app.about", &["<Ctrl>A"]);
        app_section.append(Some(&gettext("About")), Some("app.about"));

        // Quit
        let quit = gio::SimpleAction::new("quit", None);
        app.add_action(&quit);
        quit.connect_activate(|_, _| main::quit());
        app.set_accels_for_action("app.quit", &["<Ctrl>Q"]);
        app_section.append(Some(&gettext("Quit")), Some("app.quit"));

        if gst_init_res.is_ok() {
            let _ = PlaybackPipeline::check_requirements().map_err(info_bar::show_error);

            let main_section = gio::Menu::new();
            app_menu.insert_section(0, None, &main_section);

            // Register Open action
            let open = gio::SimpleAction::new("open", None);
            app.add_action(&open);
            open.connect_activate(|_, _| main::select_media());
            main_section.append(Some(&gettext("Open media file")), Some("app.open"));
            app.set_accels_for_action("app.open", &["<Ctrl>O"]);

            let display_page: gtk::Box = builder.get_object("display-box").unwrap();
            display_page.connect_map(|_| main::switch_to(UIFocusContext::PlaybackPage));

            perspective::Dispatcher::setup(&mut this.main_ctrl.perspective, app);
            video::Dispatcher::setup(&mut this.main_ctrl.video, app);
            info::Dispatcher::setup(&mut this.main_ctrl.info, app);
            info_bar::Dispatcher::setup(&mut this.main_ctrl.info_bar, app);
            audio::Dispatcher::setup(&mut this.main_ctrl.audio, app);
            export::Dispatcher::setup(&mut this.main_ctrl.export, app);
            split::Dispatcher::setup(&mut this.main_ctrl.split, app);
            streams::Dispatcher::setup(&mut this.main_ctrl.streams, app);
            playback::Dispatcher::setup(&mut this.main_ctrl, app);

            main::switch_to(UIFocusContext::PlaybackPage);

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

            Self::spawn_event_handlers(this);
            main::show_all();

            if let Some(input_file) = args.input_file.to_owned() {
                main::open_media(input_file);
            }
        } else {
            // GStreamer initialization failed
            Self::spawn_event_handlers(this);
            main::show_all();

            let msg = gettext("Failed to initialize GStreamer, the application can't be used.");
            info_bar::show_error(msg);
        }
    }
}

impl Dispatcher {
    fn spawn_event_handlers(mut self) {
        spawn(async move {
            let mut ui_event_rx = UIEventChannel::take_receiver();
            while let Some(event) = ui_event_rx.next().await {
                if self.handle(event).await.is_err() {
                    break;
                }
            }
        });
    }

    async fn handle(&mut self, event: UIEvent) -> Result<(), ()> {
        use UIEvent::*;

        match event {
            Audio(event) => audio::Dispatcher::handle_event(&mut self.main_ctrl, event).await,
            Export(event) => export::Dispatcher::handle_event(&mut self.main_ctrl, event).await,
            Info(event) => info::Dispatcher::handle_event(&mut self.main_ctrl, event).await,
            InfoBar(event) => info_bar::Dispatcher::handle_event(&mut self.main_ctrl, event).await,
            Main(event) => {
                use main::Event::*;
                match event {
                    About => self.main_ctrl.about(),
                    CancelSelectMedia => self.main_ctrl.cancel_select_media(),
                    OpenMedia(path) => self.main_ctrl.open_media(path).await,
                    Quit => {
                        self.main_ctrl.quit();
                        return Err(());
                    }
                    ResetCursor => self.reset_cursor(),
                    RestoreContext => self.restore_context(),
                    SelectMedia => self.main_ctrl.select_media().await,
                    SetCursorDoubleArrow => self.set_cursor_double_arrow(),
                    SetCursorWaiting => self.set_cursor_waiting(),
                    ShowAll => self.show_all(),
                    SwitchTo(focus_ctx) => self.switch_to(focus_ctx),
                    TemporarilySwitchTo(focus_ctx) => {
                        self.save_context();
                        self.bind_accels_for(focus_ctx);
                    }
                    UpdateFocus => self.update_focus(self.focus),
                }
            }
            Playback(event) => playback::Dispatcher::handle_event(&mut self.main_ctrl, event).await,
            Split(event) => split::Dispatcher::handle_event(&mut self.main_ctrl, event).await,
            Streams(event) => streams::Dispatcher::handle_event(&mut self.main_ctrl, event).await,
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
        perspective::Dispatcher::bind_accels_for(ctx, &self.app);
        video::Dispatcher::bind_accels_for(ctx, &self.app);
        info::Dispatcher::bind_accels_for(ctx, &self.app);
        info_bar::Dispatcher::bind_accels_for(ctx, &self.app);
        audio::Dispatcher::bind_accels_for(ctx, &self.app);
        export::Dispatcher::bind_accels_for(ctx, &self.app);
        split::Dispatcher::bind_accels_for(ctx, &self.app);
        streams::Dispatcher::bind_accels_for(ctx, &self.app);
        playback::Dispatcher::bind_accels_for(ctx, &self.app);
    }

    fn update_focus(&mut self, ctx: UIFocusContext) {
        use UIFocusContext::*;

        if self.focus == PlaybackPage && ctx != PlaybackPage {
            self.main_ctrl.info.loose_focus();
        }

        match ctx {
            ExportPage => self.main_ctrl.export.grab_focus(),
            PlaybackPage => self.main_ctrl.info.grab_focus(),
            SplitPage => self.main_ctrl.split.grab_focus(),
            StreamsPage => self.main_ctrl.streams.grab_focus(),
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
