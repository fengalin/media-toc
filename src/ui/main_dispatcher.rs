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
    pub fn setup(gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>) {
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

        if gstreamer::init().is_ok() {
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
                    match main_ctrl.state {
                        ControllerState::Playing | ControllerState::EOS => {
                            main_ctrl.hold();
                            main_ctrl.state = ControllerState::PendingSelectMedia;
                        }
                        _ => main_ctrl.select_media(),
                    }
                }
            ));
            gtk_app.set_accels_for_action("app.open", &["<Ctrl>O"]);
            main_section.append(Some(&gettext("Open media file")), Some("app.open"));

            main_ctrl.open_btn.set_sensitive(true);

            let window_box = main_ctrl.window.clone();
            let ui_event_box = main_ctrl.get_ui_event_sender();
            main_ctrl.select_media_async = Some(Box::new(move || {
                let window = window_box.clone();
                let ui_event = ui_event_box.clone();
                gtk::idle_add(move || {
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
                        let path = file_dlg.get_filename().map(|path| path.to_owned());
                        match path {
                            Some(path) => {
                                ui_event.set_cursor_waiting();
                                ui_event.open_media(path);
                            }
                            None => ui_event.cancel_select_media(),
                        }
                    } else {
                        ui_event.cancel_select_media();
                    }

                    file_dlg.close();

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

    fn handle_ui_event(main_ctrl: &mut MainController, event: UIEvent) {
        match event {
            UIEvent::AskQuestion {
                question,
                response_cb,
            } => main_ctrl.show_question(&question, response_cb),
            UIEvent::CancelSelectMedia => main_ctrl.cancel_select_media(),
            UIEvent::OpenMedia(path) => main_ctrl.open_media(path),
            UIEvent::PlayRange {
                start,
                end,
                pos_to_restore,
            } => {
                main_ctrl.play_range(start, end, pos_to_restore);
            }
            UIEvent::ResetCursor => main_ctrl.reset_cursor(),
            UIEvent::ShowAll => main_ctrl.show_all(),
            UIEvent::Seek { position, flags } => main_ctrl.seek(position, flags),
            UIEvent::SetCursorDoubleArrow => main_ctrl.set_cursor_double_arrow(),
            UIEvent::SetCursorWaiting => main_ctrl.set_cursor_waiting(),
            UIEvent::ShowError(msg) => main_ctrl.show_error(&msg),
            UIEvent::ShowInfo(msg) => main_ctrl.show_info(&msg),
        }
    }
}
