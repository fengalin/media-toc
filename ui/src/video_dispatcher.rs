use gettextrs::gettext;
use glib::clone;
use gtk::prelude::*;
use log::error;

use super::{spawn, UIDispatcher, UIEventSender, VideoController};

pub struct VideoDispatcher;
impl UIDispatcher for VideoDispatcher {
    type Controller = VideoController;

    fn setup(video_ctrl: &mut VideoController, _app: &gtk::Application, ui_event: &UIEventSender) {
        match video_ctrl.video_output {
            Some(ref video_output) => {
                // discard GStreamer defined navigation events on widget
                video_output
                    .widget
                    .set_events(gdk::EventMask::BUTTON_PRESS_MASK);

                video_ctrl.container.connect_button_press_event(
                    clone!(@strong ui_event => move |_, _| {
                        ui_event.play_pause();
                        Inhibit(true)
                    }),
                );
            }
            None => {
                error!("{}", gettext("Couldn't find GStreamer GTK video sink."));
                let container = video_ctrl.container.clone();
                spawn(async move {
                    container.hide();
                });
            }
        };
    }
}
