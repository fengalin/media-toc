use gtk::{gdk, prelude::*};
use log::error;

use crate::{playback, prelude::*, spawn, video};
use application::gettext;

pub enum Event {}

pub struct Dispatcher;
impl UIDispatcher for Dispatcher {
    type Controller = video::Controller;
    type Event = Event;

    fn setup(video: &mut video::Controller, _app: &gtk::Application) {
        match video.video_output {
            Some(ref video_output) => {
                // discard GStreamer defined navigation events on widget
                video_output
                    .widget
                    .set_events(gdk::EventMask::BUTTON_PRESS_MASK);

                video.container.connect_button_press_event(|_, _| {
                    playback::play_pause();
                    Inhibit(true)
                });
            }
            None => {
                error!("{}", gettext("Couldn't find GStreamer GTK video sink."));
                let container = video.container.clone();
                spawn(async move {
                    container.hide();
                });
            }
        };
    }
}
