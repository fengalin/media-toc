use gdk;
use gettextrs::gettext;
use gtk;
use gtk::prelude::*;
use log::error;

use std::{cell::RefCell, rc::Rc};

use crate::with_main_ctrl;

use super::{MainController, UIDispatcher, UIEventSender, VideoController};

pub struct VideoDispatcher;
impl UIDispatcher for VideoDispatcher {
    type Controller = VideoController;

    fn setup(
        video_ctrl: &mut VideoController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        _app: &gtk::Application,
        _ui_event_sender: &UIEventSender,
    ) {
        match video_ctrl.video_output {
            Some(ref video_output) => {
                // discard GStreamer defined navigation events on widget
                video_output
                    .widget
                    .set_events(gdk::EventMask::BUTTON_PRESS_MASK);

                video_ctrl
                    .container
                    .connect_button_press_event(with_main_ctrl!(
                        main_ctrl_rc => move |&mut main_ctrl, _, _event_button| {
                            main_ctrl.play_pause();
                            Inhibit(true)
                        }
                    ));
            }
            None => {
                error!("{}", gettext("Couldn't find GStreamer GTK video sink."));
                let container_clone = video_ctrl.container.clone();
                gtk::idle_add(move || {
                    container_clone.hide();
                    glib::Continue(false)
                });
            }
        };
    }
}
