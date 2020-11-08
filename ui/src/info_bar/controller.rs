use futures::channel::oneshot;

use gettextrs::gettext;

use gio::prelude::*;
use glib::clone;
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

        let close_info_bar_action = gio::SimpleAction::new("close_info_bar", None);
        app.add_action(&close_info_bar_action);
        app.set_accels_for_action("app.close_info_bar", &["Escape"]);

        let ui_event_clone = ui_event.clone();
        info_bar.connect_response(move |_, _| {
            ui_event_clone.hide_info_bar();
            ui_event_clone.restore_context();
        });

        if gst::init().is_ok() {
            close_info_bar_action
                .connect_activate(clone!(@strong info_bar => move |_, _| info_bar.emit_close()));
        } else {
            close_info_bar_action.connect_activate(clone!(@strong ui_event => move |_, _| {
                ui_event.quit();
            }));

            info_bar.connect_response(clone!(@strong ui_event => move |_, _| {
                ui_event.quit();
            }));
        }

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

    pub fn show_message(&mut self, type_: gtk::MessageType, msg: &str) {
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
        self.label.set_label(msg);
        self.revealer.set_reveal_child(true);

        self.ui_event.temporarily_switch_to(UIFocusContext::InfoBar);
    }

    pub fn show_error(&mut self, msg: &str) {
        error!("{}", msg);
        self.show_message(gtk::MessageType::Error, msg);
    }

    pub fn show_info(&mut self, msg: &str) {
        info!("{}", msg);
        self.show_message(gtk::MessageType::Info, msg);
    }

    pub fn ask_question(
        &mut self,
        question: &str,
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
            response_sender
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
