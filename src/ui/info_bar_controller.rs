use futures::channel::oneshot;

use gettextrs::gettext;

use gio;
use gio::prelude::*;
use gtk;
use gtk::prelude::*;

use log::{error, info};

use std::cell::RefCell;

use super::{UIEventSender, UIFocusContext};

pub struct InfoBarController {
    info_bar: gtk::InfoBar,
    revealer: gtk::Revealer,
    ok_btn: gtk::Button,
    label: gtk::Label,
    btn_box: gtk::ButtonBox,
    response_src: Option<glib::signal::SignalHandlerId>,
    ui_event: UIEventSender,
}

impl InfoBarController {
    pub fn new(app: &gtk::Application, builder: &gtk::Builder, ui_event: &UIEventSender) -> Self {
        let info_bar: gtk::InfoBar = builder.get_object("info_bar").unwrap();
        let ok_btn = info_bar
            .add_button(&gettext("Yes"), gtk::ResponseType::Yes)
            .unwrap();
        info_bar.add_button(&gettext("No"), gtk::ResponseType::No);
        info_bar.add_button(&gettext("Yes to all"), gtk::ResponseType::Apply);
        info_bar.add_button(&gettext("Cancel"), gtk::ResponseType::Cancel);
        info_bar.set_default_response(gtk::ResponseType::Yes);

        let revealer: gtk::Revealer = builder.get_object("info_bar-revealer").unwrap();

        let close_info_bar = gio::SimpleAction::new("close_info_bar", None);
        app.add_action(&close_info_bar);
        if gstreamer::init().is_ok() {
            let info_bar = info_bar.clone();
            close_info_bar.connect_activate(move |_, _| info_bar.emit_close());
        } else {
            let ui_event_clone = ui_event.clone();
            close_info_bar.connect_activate(move |_, _| ui_event_clone.quit());
            app.set_accels_for_action("app.close_info_bar", &["Escape"]);

            // FIXME isn't this redundant with close_info_bar.connect_activate?
            let ui_event_clone = ui_event.clone();
            info_bar.connect_response(move |_, _| ui_event_clone.quit());
        }

        let revealer_clone = revealer.clone();
        let ui_event_clone = ui_event.clone();
        info_bar.connect_response(move |_, _| {
            revealer_clone.set_reveal_child(false);
            ui_event_clone.restore_context();
        });

        let ui_event = ui_event.clone();
        InfoBarController {
            info_bar,
            revealer,
            ok_btn,
            label: builder.get_object("info_bar-lbl").unwrap(),
            btn_box: builder.get_object("info_bar-btnbox").unwrap(),
            response_src: None,
            ui_event,
        }
    }

    pub fn hide(&self) {
        self.revealer.set_reveal_child(false);
    }

    pub fn show_message<Msg: AsRef<str>>(&mut self, type_: gtk::MessageType, message: Msg) {
        if type_ == gtk::MessageType::Question {
            self.btn_box.set_visible(true);
            self.info_bar.set_show_close_button(false);
        } else {
            if let Some(src) = self.response_src.take() {
                self.info_bar.disconnect(src);
            }
            self.btn_box.set_visible(false);
            self.info_bar.set_show_close_button(true);
        }

        self.info_bar.set_message_type(type_);
        self.label.set_label(message.as_ref());
        self.revealer.set_reveal_child(true);

        self.ui_event.temporarily_switch_to(UIFocusContext::InfoBar);
    }

    pub fn show_error<Msg: AsRef<str>>(&mut self, message: Msg) {
        error!("{}", message.as_ref());
        self.show_message(gtk::MessageType::Error, message);
    }

    pub fn show_info<Msg: AsRef<str>>(&mut self, message: Msg) {
        info!("{}", message.as_ref());
        self.show_message(gtk::MessageType::Info, message);
    }

    pub fn ask_question<Q: AsRef<str>>(
        &mut self,
        question: Q,
        response_sender: oneshot::Sender<gtk::ResponseType>,
    ) {
        let revealer = self.revealer.clone();
        if let Some(src) = self.response_src.take() {
            self.info_bar.disconnect(src);
        }

        let ui_event = self.ui_event.clone();
        let response_sender = RefCell::new(Some(response_sender));
        self.response_src = Some(self.info_bar.connect_response(move |_, response_type| {
            revealer.set_reveal_child(false);
            ui_event.restore_context();
            let _ = response_sender
                .borrow_mut()
                .take()
                .unwrap()
                .send(response_type)
                .expect("UI failed to send response");
        }));
        self.ok_btn.grab_default();
        self.show_message(gtk::MessageType::Question, question);
    }
}
