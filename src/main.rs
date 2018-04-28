extern crate byteorder;
extern crate cairo;
extern crate clap;
extern crate env_logger;
extern crate gdk;
extern crate gettextrs;
extern crate glib;
extern crate gstreamer;
extern crate gstreamer_audio;
extern crate gtk;
extern crate image;
extern crate pango;
extern crate sample;

#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[macro_use]
extern crate nom;

#[cfg(feature = "dump-waveform")]
extern crate chrono;

use clap::{App, Arg};

use gettextrs::{gettext, TextDomain, TextDomainError};

use gtk::Builder;

mod metadata;
mod media;
mod ui;
use ui::MainController;

fn main() {
    env_logger::init();

    match TextDomain::new("media-toc").prepend("target").init() {
        Ok(locale) => info!("Translation found, `setlocale` returned {:?}", locale),
        Err(TextDomainError::TranslationNotFound(lang)) => {
            warn!("Translation not found for language {}", lang)
        }
        Err(TextDomainError::InvalidLocale(locale)) => error!("Invalid locale {}", locale),
    }

    // Messages are not translated unless gtk (glib) is initialized
    let is_gtk_ok = gtk::init().is_ok();

    let about_msg =
        gettext("Build a table of contents from a media file\nor split a media file into chapters");
    let help_msg = gettext("Display this message");
    let version_msg = gettext("Print version information");

    let input_arg = gettext("MEDIA");

    let matches = App::new("media-toc")
        .version("0.4.1")
        .author("François Laignel <fengalin@free.fr>")
        .about(about_msg.as_str())
        .help_message(help_msg.as_str())
        .version_message(version_msg.as_str())
        .arg(
            Arg::with_name(input_arg.as_str())
                .help(&gettext("Path to the input media file"))
                .last(false),
        )
        .get_matches();

    if !is_gtk_ok {
        error!("{}", gettext("Failed to initialize GTK"));
        return;
    }

    // TODO: there's a `Settings` struct in GTK:
    // https://github.com/gtk-rs/gtk/blob/master/src/auto/settings.rs

    let is_gst_ok = gstreamer::init().is_ok();
    let main_ctrl = MainController::new(
        &Builder::new_from_string(include_str!("ui/media-toc.ui")),
        is_gst_ok,
    );
    main_ctrl.borrow().show_all();

    if is_gst_ok {
        if let Some(input_file) = matches.value_of(input_arg.as_str()) {
            main_ctrl.borrow_mut().open_media(input_file.into());
        }
    }

    gtk::main();
}