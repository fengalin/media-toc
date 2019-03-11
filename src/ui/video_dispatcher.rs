use gdk;
use gettextrs::gettext;
use gtk;
use gtk::prelude::*;
use log::error;

use std::{cell::RefCell, rc::Rc};

use super::{MainController, UIDispatcher};

pub struct VideoDispatcher;
impl UIDispatcher for VideoDispatcher {
    fn setup(_gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>) {
        let main_ctrl = main_ctrl_rc.borrow();

        match main_ctrl.video_ctrl.video_output {
            Some(ref video_output) => {
                // discard GStreamer defined navigation events on widget
                video_output
                    .widget
                    .set_events(gdk::EventMask::BUTTON_PRESS_MASK);

                let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
                main_ctrl.video_ctrl.container.connect_button_press_event(
                    move |_, _event_button| {
                        main_ctrl_rc_cb.borrow_mut().play_pause();
                        Inhibit(true)
                    },
                );
            }
            None => {
                error!("{}", gettext("Couldn't find GStreamer GTK video sink."));
                let container_clone = main_ctrl.video_ctrl.container.clone();
                gtk::idle_add(move || {
                    container_clone.hide();
                    glib::Continue(false)
                });
            }
        };
    }
}
