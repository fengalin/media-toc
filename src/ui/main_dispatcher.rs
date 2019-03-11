use gettextrs::gettext;
use gio;
use gio::prelude::*;

use glib;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use crate::media::PlaybackPipeline;

use super::{
    main_controller::ControllerState, AudioDispatcher, ExportDispatcher, InfoDispatcher,
    MainController, PerspectiveDispatcher, SplitDispatcher, StreamsDispatcher, UIDispatcher,
    VideoDispatcher,
};

pub struct MainDispatcher;
impl MainDispatcher {
    pub fn setup(
        gtk_app: &gtk::Application,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        is_gst_ok: bool,
    ) {
        main_ctrl_rc.borrow_mut().window.set_application(gtk_app);

        let app_menu = gio::Menu::new();
        gtk_app.set_app_menu(&app_menu);

        let app_section = gio::Menu::new();
        app_menu.append_section(None, &app_section);

        // Register About action
        let about = gio::SimpleAction::new("about", None);
        gtk_app.add_action(&about);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        about.connect_activate(move |_, _| main_ctrl_rc_cb.borrow().about());
        gtk_app.set_accels_for_action("app.about", &["<Ctrl>A"]);
        app_section.append(&gettext("About")[..], "app.about");

        // Register Quit action
        let quit = gio::SimpleAction::new("quit", None);
        gtk_app.add_action(&quit);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        quit.connect_activate(move |_, _| main_ctrl_rc_cb.borrow_mut().quit());
        gtk_app.set_accels_for_action("app.quit", &["<Ctrl>Q"]);
        app_section.append(&gettext("Quit")[..], "app.quit");

        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        main_ctrl_rc
            .borrow_mut()
            .window
            .connect_delete_event(move |_, _| {
                main_ctrl_rc_cb.borrow_mut().quit();
                Inhibit(false)
            });

        if is_gst_ok {
            PerspectiveDispatcher::setup(gtk_app, main_ctrl_rc);
            VideoDispatcher::setup(gtk_app, main_ctrl_rc);
            InfoDispatcher::setup(gtk_app, main_ctrl_rc);
            AudioDispatcher::setup(gtk_app, main_ctrl_rc);
            ExportDispatcher::setup(gtk_app, main_ctrl_rc);
            SplitDispatcher::setup(gtk_app, main_ctrl_rc);
            StreamsDispatcher::setup(gtk_app, main_ctrl_rc);

            let mut main_ctrl = main_ctrl_rc.borrow_mut();

            let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
            main_ctrl.media_event_handler = Some(Rc::new(move |event| {
                main_ctrl_rc_cb.borrow_mut().handle_media_event(event)
            }));

            let _ = PlaybackPipeline::check_requirements().map_err(|err| {
                let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
                gtk::idle_add(move || {
                    main_ctrl_rc_cb.borrow().show_error(&err);
                    glib::Continue(false)
                });
            });

            let main_section = gio::Menu::new();
            app_menu.insert_section(0, None, &main_section);

            // Register Open action
            let open = gio::SimpleAction::new("open", None);
            gtk_app.add_action(&open);
            let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
            open.connect_activate(move |_, _| {
                let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();

                if main_ctrl.state == ControllerState::Playing
                    || main_ctrl.state == ControllerState::EOS
                {
                    main_ctrl.hold();
                    main_ctrl.state = ControllerState::PendingSelectMedia;
                } else {
                    main_ctrl.select_media();
                }
            });
            gtk_app.set_accels_for_action("app.open", &["<Ctrl>O"]);
            main_section.append(&gettext("Open media file")[..], "app.open");

            main_ctrl.open_btn.set_sensitive(true);

            // Register Play/Pause action
            let play_pause = gio::SimpleAction::new("play_pause", None);
            gtk_app.add_action(&play_pause);
            let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
            play_pause.connect_activate(move |_, _| {
                main_ctrl_rc_cb.borrow_mut().play_pause();
            });
            gtk_app.set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);

            main_ctrl.play_pause_btn.set_sensitive(true);

            // Register Close info bar action
            let close_info_bar = gio::SimpleAction::new("close_info_bar", None);
            gtk_app.add_action(&close_info_bar);
            let revealer = main_ctrl.info_bar_revealer.clone();
            close_info_bar.connect_activate(move |_, _| revealer.set_reveal_child(false));
            gtk_app.set_accels_for_action("app.close_info_bar", &["Escape"]);

            let revealer = main_ctrl.info_bar_revealer.clone();
            main_ctrl
                .info_bar
                .connect_response(move |_, _| revealer.set_reveal_child(false));
        } else {
            // GStreamer initialization failed
            let main_ctrl = main_ctrl_rc.borrow();

            // Register Close info bar action
            let close_info_bar = gio::SimpleAction::new("close_info_bar", None);
            gtk_app.add_action(&close_info_bar);
            let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
            close_info_bar.connect_activate(move |_, _| main_ctrl_rc_cb.borrow_mut().quit());
            gtk_app.set_accels_for_action("app.close_info_bar", &["Escape"]);

            let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
            main_ctrl
                .info_bar
                .connect_response(move |_, _| main_ctrl_rc_cb.borrow_mut().quit());

            let msg = gettext("Failed to initialize GStreamer, the application can't be used.");
            main_ctrl.show_error(msg);
        }
    }
}
