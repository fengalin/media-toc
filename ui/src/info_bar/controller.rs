use futures::channel::oneshot;
use gtk::{glib, prelude::*};
use log::{error, info};

use std::cell::RefCell;

use crate::{info_bar, main, prelude::*};
use application::gettext;

pub struct Controller {
    pub(super) info_bar: gtk::InfoBar,
    revealer: gtk::Revealer,
    ok_btn: gtk::Button,
    label: gtk::Label,
    btn_box: gtk::ButtonBox,
    response_src: Option<glib::signal::SignalHandlerId>,
}

impl UIController for Controller {
    fn cleanup(&mut self) {
        self.hide();
    }
}

impl Controller {
    pub fn new(builder: &gtk::Builder) -> Self {
        let info_bar: gtk::InfoBar = builder.object("info_bar").unwrap();
        let ok_btn = info_bar
            .add_button(&gettext("Yes"), gtk::ResponseType::Yes)
            .unwrap();
        info_bar.add_button(&gettext("No"), gtk::ResponseType::No);
        info_bar.add_button(&gettext("Yes to all"), gtk::ResponseType::Apply);
        info_bar.add_button(&gettext("Cancel"), gtk::ResponseType::Cancel);
        info_bar.set_default_response(gtk::ResponseType::Yes);

        let revealer: gtk::Revealer = builder.object("info_bar-revealer").unwrap();

        info_bar::Controller {
            info_bar,
            revealer,
            ok_btn,
            label: builder.object("info_bar-lbl").unwrap(),
            btn_box: builder.object("info_bar-btnbox").unwrap(),
            response_src: None,
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

        main::temporarily_switch_to(UIFocusContext::InfoBar);
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
        if let Some(src) = self.response_src.take() {
            self.info_bar.disconnect(src);
        }

        let revealer = self.revealer.clone();
        let response_sender = RefCell::new(Some(response_sender));
        self.response_src = Some(self.info_bar.connect_response(move |_, response_type| {
            revealer.set_reveal_child(false);
            main::restore_context();
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
