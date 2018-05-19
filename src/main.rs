extern crate byteorder;
extern crate cairo;
extern crate clap;
extern crate directories;
extern crate env_logger;
extern crate gdk;
extern crate gettextrs;
extern crate gio;
extern crate glib;
extern crate gstreamer;
extern crate gstreamer_audio;
extern crate gtk;
extern crate image;
extern crate pango;
extern crate ron;
extern crate sample;
extern crate serde;

#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[macro_use]
extern crate nom;
#[macro_use]
extern crate serde_derive;

#[cfg(feature = "dump-waveform")]
extern crate chrono;

use gettextrs::gettext;

mod application;
use application::{handle_command_line, init_locale, run};
mod media;
mod metadata;
mod ui;

fn main() {
    env_logger::init();

    init_locale();

    // Messages are not translated unless gtk (glib) is initialized
    let is_gtk_ok = gtk::init().is_ok();

    let args = handle_command_line();

    if is_gtk_ok {
        run(gstreamer::init().is_ok(), args);
    } else {
        error!("{}", gettext("Failed to initialize GTK"));
    }
}