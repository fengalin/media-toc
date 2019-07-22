use gettextrs::gettext;
use gio;
use gio::prelude::*;

use glib;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use super::{
    main_controller::ControllerState, AudioDispatcher, ExportDispatcher, InfoDispatcher,
    MainController, PerspectiveDispatcher, PlaybackPipeline, SplitDispatcher, StreamsDispatcher,
    UIDispatcher, UIEvent, VideoDispatcher,
};
use crate::{application::CONFIG, with_main_ctrl};

pub struct MainDispatcher;
impl MainDispatcher {
    pub fn setup(
        main_ctrl: &mut MainController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        ui_event_receiver: glib::Receiver<UIEvent>,
    ) {
        ui_event_receiver.attach(
            None,
            with_main_ctrl!(main_ctrl_rc => move |&mut main_ctrl, event| {
                Self::handle_ui_event(&mut main_ctrl, event);
                glib::Continue(true)
            }),
        );

        let app_menu = gio::Menu::new();
        main_ctrl.app.set_app_menu(Some(&app_menu));

        let app_section = gio::Menu::new();
        app_menu.append_section(None, &app_section);

        // About
        let about = gio::SimpleAction::new("about", None);
        main_ctrl.app.add_action(&about);
        about.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&main_ctrl, _, _| main_ctrl.about()
        ));
        main_ctrl
            .app
            .set_accels_for_action("app.about", &["<Ctrl>A"]);
        app_section.append(Some(&gettext("About")), Some("app.about"));

        // Quit
        let quit = gio::SimpleAction::new("quit", None);
        main_ctrl.app.add_action(&quit);
        quit.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| main_ctrl.quit()
        ));
        main_ctrl
            .app
            .set_accels_for_action("app.quit", &["<Ctrl>Q"]);
        app_section.append(Some(&gettext("Quit")), Some("app.quit"));

        main_ctrl.window.connect_delete_event(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| {
                main_ctrl.quit();
                Inhibit(false)
            }
        ));

        if gstreamer::init().is_ok() {
            let app = main_ctrl.app.clone();
            PerspectiveDispatcher::setup(&mut main_ctrl.perspective_ctrl, main_ctrl_rc, &app);
            VideoDispatcher::setup(&mut main_ctrl.video_ctrl, main_ctrl_rc, &app);
            InfoDispatcher::setup(&mut main_ctrl.info_ctrl, main_ctrl_rc, &app);
            AudioDispatcher::setup(&mut main_ctrl.audio_ctrl, main_ctrl_rc, &app);
            ExportDispatcher::setup(&mut main_ctrl.export_ctrl, main_ctrl_rc, &app);
            SplitDispatcher::setup(&mut main_ctrl.split_ctrl, main_ctrl_rc, &app);
            StreamsDispatcher::setup(&mut main_ctrl.streams_ctrl, main_ctrl_rc, &app);

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
            main_ctrl.app.add_action(&open);
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
            main_ctrl
                .app
                .set_accels_for_action("app.open", &["<Ctrl>O"]);
            main_section.append(Some(&gettext("Open media file")), Some("app.open"));

            main_ctrl.open_btn.set_sensitive(true);

            let window_box = main_ctrl.window.clone();
            let ui_event_box = main_ctrl.ui_event_sender();
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
            main_ctrl.app.add_action(&play_pause);
            play_pause.connect_activate(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _| {
                    main_ctrl.play_pause();
                }
            ));
            main_ctrl
                .app
                .set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);

            main_ctrl.play_pause_btn.set_sensitive(true);

            // Register Close info bar action
            let close_info_bar = gio::SimpleAction::new("close_info_bar", None);
            main_ctrl.app.add_action(&close_info_bar);
            let info_bar = main_ctrl.info_bar.clone();
            close_info_bar.connect_activate(move |_, _| info_bar.emit_close());

            // FIXME: register/unregister when opening/closing info bar
            main_ctrl
                .app
                .set_accels_for_action("app.close_info_bar", &["Escape"]);

            let revealer = main_ctrl.info_bar_revealer.clone();
            main_ctrl
                .info_bar
                .connect_response(move |_, _| revealer.set_reveal_child(false));

            // FIXME: implement actual accell activation/deactivation in InfoDispatcher
            // activate Next chapter accels only when the display page is shown
            // so that other pages can use the Up & Down keys
            let app_cb = app.clone();
            main_ctrl.display_page.connect_map(move |_| {
                app_cb.set_accels_for_action("app.next_chapter", &["Down", "AudioNext"]);
                app_cb.set_accels_for_action("app.previous_chapter", &["Up", "AudioPrev"]);
            });

            let app_cb = app.clone();
            main_ctrl.display_page.connect_unmap(move |_| {
                app_cb.set_accels_for_action("app.next_chapter", &["AudioNext"]);
                app_cb.set_accels_for_action("app.previous_chapter", &["AudioPrev"]);
            });
        } else {
            // GStreamer initialization failed
            let mut main_ctrl = main_ctrl_rc.borrow_mut();

            // Register Close info bar action
            let close_info_bar = gio::SimpleAction::new("close_info_bar", None);
            main_ctrl.app.add_action(&close_info_bar);
            close_info_bar.connect_activate(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _| main_ctrl.quit()
            ));
            main_ctrl
                .app
                .set_accels_for_action("app.close_info_bar", &["Escape"]);

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
                ts_to_restore,
            } => {
                main_ctrl.play_range(start, end, ts_to_restore);
            }
            UIEvent::ResetCursor => main_ctrl.reset_cursor(),
            UIEvent::ShowAll => main_ctrl.show_all(),
            UIEvent::Seek { target, flags } => main_ctrl.seek(target, flags),
            UIEvent::SetCursorDoubleArrow => main_ctrl.set_cursor_double_arrow(),
            UIEvent::SetCursorWaiting => main_ctrl.set_cursor_waiting(),
            UIEvent::ShowError(msg) => main_ctrl.show_error(&msg),
            UIEvent::ShowInfo(msg) => main_ctrl.show_info(&msg),
        }
    }
}
