use futures::channel::mpsc as async_mpsc;
use futures::prelude::*;

use gettextrs::gettext;

use gio;
use gio::prelude::*;
use glib::clone;
use gtk;
use gtk::prelude::*;

use log::debug;

use std::{cell::RefCell, rc::Rc};

use media::MediaEvent;

use super::{
    main_controller::ControllerState, AudioDispatcher, ExportDispatcher, InfoDispatcher,
    MainController, PerspectiveDispatcher, PlaybackPipeline, SplitDispatcher, StreamsDispatcher,
    UIDispatcher, UIFocusContext, VideoDispatcher,
};

pub struct MainDispatcher;
impl MainDispatcher {
    pub fn setup(
        main_ctrl: &mut MainController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        app: &gtk::Application,
    ) {
        let app_menu = gio::Menu::new();
        app.set_app_menu(Some(&app_menu));

        let app_section = gio::Menu::new();
        app_menu.append_section(None, &app_section);

        // About
        let about = gio::SimpleAction::new("about", None);
        app.add_action(&about);
        about.connect_activate(clone!(@strong main_ctrl_rc => move |_, _| {
            main_ctrl_rc.borrow().about();
        }));
        app.set_accels_for_action("app.about", &["<Ctrl>A"]);
        app_section.append(Some(&gettext("About")), Some("app.about"));

        // Quit
        let quit = gio::SimpleAction::new("quit", None);
        app.add_action(&quit);
        quit.connect_activate(clone!(@strong main_ctrl_rc => move |_, _| {
            main_ctrl_rc.borrow_mut().quit();
        }));
        app.set_accels_for_action("app.quit", &["<Ctrl>Q"]);
        app_section.append(Some(&gettext("Quit")), Some("app.quit"));

        main_ctrl
            .window
            .connect_delete_event(clone!(@strong main_ctrl_rc => move |_, _| {
                main_ctrl_rc.borrow_mut().quit();
                Inhibit(false)
            }));

        let ui_event = main_ctrl.ui_event().clone();
        if gstreamer::init().is_ok() {
            main_ctrl.new_media_event_handler =
                Some(Box::new(clone!(@strong main_ctrl_rc => move |receiver| {
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
                })));

            let ui_event_clone = ui_event.clone();
            let _ = PlaybackPipeline::check_requirements()
                .map_err(move |err| ui_event_clone.show_error(err));

            let main_section = gio::Menu::new();
            app_menu.insert_section(0, None, &main_section);

            // Register Open action
            let open = gio::SimpleAction::new("open", None);
            app.add_action(&open);
            open.connect_activate(clone!(@strong main_ctrl_rc => move |_, _| {
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
            play_pause.connect_activate(clone!(@strong main_ctrl_rc => move |_, _| {
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
