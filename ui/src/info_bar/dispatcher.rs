use futures::{future::LocalBoxFuture, prelude::*};

use gio::prelude::*;
use glib::clone;
use gtk::prelude::*;

use log::debug;

use crate::{info_bar, main, prelude::*};

pub struct Dispatcher;

impl UIDispatcher for Dispatcher {
    type Controller = info_bar::Controller;
    type Event = info_bar::Event;

    fn setup(ctrl: &mut info_bar::Controller, app: &gtk::Application) {
        let close_info_bar_action = gio::SimpleAction::new("close_info_bar", None);
        app.add_action(&close_info_bar_action);
        app.set_accels_for_action("app.close_info_bar", &["Escape"]);

        ctrl.info_bar.connect_response(|_, _| {
            info_bar::hide();
            main::restore_context();
        });

        if gst::init().is_ok() {
            close_info_bar_action.connect_activate(
                clone!(@strong ctrl.info_bar as info_bar => move |_, _| info_bar.emit_close()),
            );
        } else {
            close_info_bar_action.connect_activate(|_, _| main::quit());

            ctrl.info_bar.connect_response(|_, _| main::quit());
        }
    }

    fn handle_event(
        main_ctrl: &mut main::Controller,
        event: impl Into<Self::Event>,
    ) -> LocalBoxFuture<'_, ()> {
        let event = event.into();
        async move {
            use info_bar::Event::*;

            debug!("handling {:?}", event);
            match event {
                AskQuestion {
                    question,
                    response_sender,
                } => main_ctrl.info_bar.ask_question(&question, response_sender),
                Hide => main_ctrl.info_bar.hide(),
                ShowError(msg) => main_ctrl.info_bar.show_error(&msg),
                ShowInfo(msg) => main_ctrl.info_bar.show_info(&msg),
            }
        }
        .boxed_local()
    }

    fn bind_accels_for(ctx: UIFocusContext, app: &gtk::Application) {
        use UIFocusContext::*;

        match ctx {
            PlaybackPage => {
                app.set_accels_for_action("app.close_info_bar", &[]);
            }
            ExportPage | SplitPage | StreamsPage => {
                app.set_accels_for_action("app.close_info_bar", &[]);
            }
            TextEntry => {
                app.set_accels_for_action("app.close_info_bar", &[]);
            }
            InfoBar => {
                app.set_accels_for_action("app.close_info_bar", &["Escape"]);
            }
        }
    }
}
