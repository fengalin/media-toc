use gdk;
use gettextrs::gettext;
use glib::clone;
use gtk;
use gtk::prelude::*;
use log::error;

use std::{cell::RefCell, rc::Rc};

use crate::spawn;

use super::{MainController, UIDispatcher, UIEventSender, VideoController};

pub struct VideoDispatcher;
impl UIDispatcher for VideoDispatcher {
    type Controller = VideoController;

    fn setup(
        video_ctrl: &mut VideoController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        _app: &gtk::Application,
        _ui_event: &UIEventSender,
    ) {
        match video_ctrl.video_output {
            Some(ref video_output) => {
                // discard GStreamer defined navigation events on widget
                video_output
                    .widget
                    .set_events(gdk::EventMask::BUTTON_PRESS_MASK);

                video_ctrl.container.connect_button_press_event(
                    clone!(@strong main_ctrl_rc => move |_, _| {
                        main_ctrl_rc.borrow_mut().play_pause();
                        Inhibit(true)
                    }),
                );
            }
            None => {
                error!("{}", gettext("Couldn't find GStreamer GTK video sink."));
                let container = video_ctrl.container.clone();
                spawn!(async move {
                    container.hide();
                });
            }
        };
    }
}
