extern crate gtk;

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;
use gtk::{ApplicationWindow, HeaderBar, Statusbar};

use video_controller::VideoController;
use audio_controller::AudioController;

pub struct MainController {
    window: ApplicationWindow,
    header_bar: HeaderBar,
    status_bar: Statusbar,
    video_ctrl: Rc<RefCell<VideoController>>,
    audio_ctrl: Rc<RefCell<AudioController>>,
}

impl MainController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<MainController>> {
        let mc = Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").expect("Couldn't find application-window"),
            header_bar: builder.get_object("header-bar").expect("Couldn't find header-bar"),
            status_bar: builder.get_object("status-bar").expect("Couldn't find status-bar"),
            video_ctrl: VideoController::new(&builder),
            audio_ctrl: AudioController::new(&builder),
        }));

        {
            let mc_ref = mc.borrow();
            mc_ref.window.connect_delete_event(|_, _| {
                gtk::main_quit();
                Inhibit(false)
            });
            mc_ref.window.set_titlebar(&mc_ref.header_bar);
            mc_ref.display_something();
        }

        mc
    }

    fn display_something(&self) {
        self.status_bar.push(self.status_bar.get_context_id("dummy msg"),
                             "Media-TOC prototype");
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }
}
