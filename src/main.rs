extern crate byte_slice_cast;
#[cfg(test)]
extern crate byteorder;

extern crate gdk;
extern crate glib;
extern crate gstreamer;
extern crate gstreamer_audio;
extern crate gtk;
extern crate pango;

#[macro_use]
extern crate lazy_static;

#[cfg(any(feature = "dump-waveform", feature = "profiling-audio-draw",
          feature = "profiling-audio-buffer", feature = "profiling-waveform-buffer",
          feature = "profile-waveform-image"))]
extern crate chrono;

use gtk::{Builder, BuilderExt};

mod metadata;
mod media;
mod ui;
use ui::MainController;

fn main() {
    if gtk::init().is_err() {
        panic!("Failed to initialize GTK.");
    }

    gstreamer::init().unwrap();

    // TODO: there's a `Settings` struct in GTK:
    // https://github.com/gtk-rs/gtk/blob/master/src/auto/settings.rs

    let main_ctrl = {
        let builder = Builder::new_from_string(include_str!("ui/media-toc.ui"));
        builder
            .add_from_string(include_str!("ui/media-toc-export.ui"))
            .unwrap();
        MainController::new(&builder)
    };
    main_ctrl.borrow().show_all();

    gtk::main();
}
