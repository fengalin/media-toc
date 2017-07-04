extern crate gtk;
extern crate cairo;

use gtk::prelude::*;
use gtk::{Builder, ApplicationWindow, HeaderBar, Statusbar};

mod video_controller;
use video_controller::VideoController;

mod audio_controller;
use audio_controller::AudioController;


fn display_something(builder: &Builder) {
    let status_bar: Statusbar = builder.get_object("status-bar").unwrap();
    status_bar.push(status_bar.get_context_id("dummy msg"), "Media-TOC prototype");
}

fn main() {
    if gtk::init().is_err() {
        panic!("Failed to initialize GTK.");
    }

    let builder = Builder::new_from_string(include_str!("media-toc.glade"));

    let window: ApplicationWindow = builder.get_object("application-window").unwrap();
    let header_bar: HeaderBar = builder.get_object("header-bar").unwrap();
    window.set_titlebar(&header_bar);

    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        Inhibit(false)
    });


    let video_ctrl = VideoController::new(&builder);
    let audio_ctrl = AudioController::new(&builder);

    window.show_all();

    display_something(&builder);

    gtk::main();
}
