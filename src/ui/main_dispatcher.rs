use gettextrs::gettext;
use gio;
use gio::prelude::*;

use glib;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use crate::{application::CONFIG, media::PlaybackPipeline, with_main_ctrl};

use super::{
    main_controller::ControllerState, AudioDispatcher, ExportDispatcher, InfoDispatcher,
    MainController, PerspectiveDispatcher, SplitDispatcher, StreamsDispatcher, UIDispatcher,
    UIEvent, VideoDispatcher,
};

pub struct MainDispatcher;
impl MainDispatcher {
    pub fn setup(
        gtk_app: &gtk::Application,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        is_gst_ok: bool,
    ) {
        {
            let mut main_ctrl = main_ctrl_rc.borrow_mut();
            main_ctrl.window.set_application(Some(gtk_app));

            main_ctrl
                .ui_event_receiver
                .take()
                .expect("MainDispatcher: `ui_event_receiver` already taken")
                .attach(
                    None,
                    with_main_ctrl!(main_ctrl_rc => move |&mut main_ctrl, event| {
                        Self::handle_ui_event(&mut main_ctrl, event);
                        glib::Continue(true)
                    }),
                );
        }

        let app_menu = gio::Menu::new();
        gtk_app.set_app_menu(Some(&app_menu));

        let app_section = gio::Menu::new();
        app_menu.append_section(None, &app_section);

        // About
        let about = gio::SimpleAction::new("about", None);
        gtk_app.add_action(&about);
        about.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&main_ctrl, _, _| main_ctrl.about()
        ));
        gtk_app.set_accels_for_action("app.about", &["<Ctrl>A"]);
        app_section.append(Some(&gettext("About")), Some("app.about"));

        // Quit
        let quit = gio::SimpleAction::new("quit", None);
        gtk_app.add_action(&quit);
        quit.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| main_ctrl.quit()
        ));
        gtk_app.set_accels_for_action("app.quit", &["<Ctrl>Q"]);
        app_section.append(Some(&gettext("Quit")), Some("app.quit"));

        main_ctrl_rc
            .borrow()
            .window
            .connect_delete_event(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _| {
                    main_ctrl.quit();
                    Inhibit(false)
                }
            ));

        if is_gst_ok {
            PerspectiveDispatcher::setup(gtk_app, main_ctrl_rc);
            VideoDispatcher::setup(gtk_app, main_ctrl_rc);
            InfoDispatcher::setup(gtk_app, main_ctrl_rc);
            AudioDispatcher::setup(gtk_app, main_ctrl_rc);
            ExportDispatcher::setup(gtk_app, main_ctrl_rc);
            SplitDispatcher::setup(gtk_app, main_ctrl_rc);
            StreamsDispatcher::setup(gtk_app, main_ctrl_rc);

            let mut main_ctrl = main_ctrl_rc.borrow_mut();

            main_ctrl.media_event_handler = Some(Rc::new(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, event| {
                    main_ctrl.handle_media_event(event)
                }
            )));

            let _ = PlaybackPipeline::check_requirements().map_err(|err| {
                with_main_ctrl!(
                    main_ctrl_rc => async move |&mut main_ctrl| {
                        main_ctrl.show_error(&err);
                        glib::Continue(false)
                    }
                )
            });

            let main_section = gio::Menu::new();
            app_menu.insert_section(0, None, &main_section);

            // Register Open action
            let open = gio::SimpleAction::new("open", None);
            gtk_app.add_action(&open);
            open.connect_activate(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _| {
                    let state = main_ctrl.state;
                    if state == ControllerState::Playing || state == ControllerState::EOS {
                        main_ctrl.hold();
                        main_ctrl.state = ControllerState::PendingSelectMedia;
                    } else {
                        main_ctrl.select_media();
                    }
                }
            ));
            gtk_app.set_accels_for_action("app.open", &["<Ctrl>O"]);
            main_section.append(Some(&gettext("Open media file")), Some("app.open"));

            main_ctrl.open_btn.set_sensitive(true);

            let main_ctrl_rc_box = Rc::clone(main_ctrl_rc);
            main_ctrl.select_media_async = Some(Box::new(move || {
                let main_ctrl_rc_idle = Rc::clone(&main_ctrl_rc_box);
                gtk::idle_add(move || {
                    MainDispatcher::select_media(&main_ctrl_rc_idle);
                    glib::Continue(false)
                });
            }));

            // Register Play/Pause action
            let play_pause = gio::SimpleAction::new("play_pause", None);
            gtk_app.add_action(&play_pause);
            play_pause.connect_activate(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _| {
                    main_ctrl.play_pause();
                }
            ));
            gtk_app.set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);

            main_ctrl.play_pause_btn.set_sensitive(true);

            // Register Close info bar action
            let close_info_bar = gio::SimpleAction::new("close_info_bar", None);
            gtk_app.add_action(&close_info_bar);
            let info_bar = main_ctrl.info_bar.clone();
            close_info_bar.connect_activate(move |_, _| info_bar.emit_close());
            gtk_app.set_accels_for_action("app.close_info_bar", &["Escape"]);

            let revealer = main_ctrl.info_bar_revealer.clone();
            main_ctrl
                .info_bar
                .connect_response(move |_, _| revealer.set_reveal_child(false));
        } else {
            // GStreamer initialization failed
            let mut main_ctrl = main_ctrl_rc.borrow_mut();

            // Register Close info bar action
            let close_info_bar = gio::SimpleAction::new("close_info_bar", None);
            gtk_app.add_action(&close_info_bar);
            close_info_bar.connect_activate(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _| main_ctrl.quit()
            ));
            gtk_app.set_accels_for_action("app.close_info_bar", &["Escape"]);

            main_ctrl.info_bar.connect_response(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _| main_ctrl.quit()
            ));

            let msg = gettext("Failed to initialize GStreamer, the application can't be used.");
            main_ctrl.show_error(msg);
        }
    }

    fn select_media(main_ctrl_rc: &Rc<RefCell<MainController>>) {
        let window = {
            let mut main_ctrl = main_ctrl_rc.borrow_mut();
            main_ctrl.info_bar_revealer.set_reveal_child(false);
            main_ctrl.switch_to_busy();
            main_ctrl.window.clone()
        };

        let file_dlg = gtk::FileChooserDialog::with_buttons(
            Some(gettext("Open a media file").as_str()),
            Some(&window),
            gtk::FileChooserAction::Open,
            &[
                (&gettext("Cancel"), gtk::ResponseType::Cancel),
                (&gettext("Open"), gtk::ResponseType::Accept),
            ],
        );
        if let Some(ref last_path) = CONFIG.read().unwrap().media.last_path {
            file_dlg.set_current_folder(last_path);
        }

        if file_dlg.run() == gtk::ResponseType::Accept {
            main_ctrl_rc
                .borrow_mut()
                .open_media(&file_dlg.get_filename().unwrap());
        } else {
            let mut main_ctrl = main_ctrl_rc.borrow_mut();
            if main_ctrl.pipeline.is_some() {
                main_ctrl.state = ControllerState::Paused;
            }
            main_ctrl.switch_to_default();
        }

        file_dlg.close();
    }

    fn handle_ui_event(main_ctrl: &mut MainController, event: UIEvent) {
        match event {
            UIEvent::AskQuestion {
                question,
                response_cb,
            } => main_ctrl.show_question(&question, response_cb),
            UIEvent::HandBackPipeline(playback_pipeline) => {
                main_ctrl.set_pipeline(playback_pipeline)
            }
            UIEvent::PlayRange {
                start,
                end,
                pos_to_restore,
            } => {
                main_ctrl.play_range(start, end, pos_to_restore);
            }
            UIEvent::ResetCursor => main_ctrl.reset_cursor(),
            UIEvent::Seek { position, flags } => main_ctrl.seek(position, flags),
            UIEvent::SetCursorWaiting => main_ctrl.set_cursor_waiting(),
            UIEvent::ShowError(msg) => main_ctrl.show_error(&msg),
            UIEvent::ShowInfo(msg) => main_ctrl.show_info(&msg),
        }
    }
}
