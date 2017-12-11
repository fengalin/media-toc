#![feature(collection_placement)]
#![feature(ord_max_min)]
#![feature(placement_in_syntax)]

extern crate byte_slice_cast;
#[cfg(test)]
extern crate byteorder;

extern crate glib;
extern crate gstreamer;
extern crate gstreamer_audio;
extern crate gtk;

#[macro_use]
extern crate lazy_static;

#[cfg(any(feature = "dump-waveform", feature = "profiling-audio-draw",
            feature = "profiling-audio-buffer", feature = "profiling-tracker",
            feature = "profiling-waveform-buffer", feature = "profile-waveform-image"))]
extern crate chrono;

use gtk::{Builder, BuilderExt};

mod ui;
use ui::MainController;

mod media;
mod toc;

fn main() {
    if gtk::init().is_err() {
        panic!("Failed to initialize GTK.");
    }

    gstreamer::init().unwrap();

    // TODO: there's a `Settings` struct in GTK:
    // https://github.com/gtk-rs/gtk/blob/master/src/auto/settings.rs

    let main_ctrl = {
        let builder = Builder::new_from_string(include_str!("ui/media-toc.ui"));
        builder.add_from_string(include_str!("ui/media-toc-export.ui")).unwrap();
        MainController::new(&builder)
    };
    main_ctrl.borrow().show_all();

    gtk::main();
}
