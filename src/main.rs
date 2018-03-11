extern crate byteorder;
extern crate cairo;
extern crate clap;
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

#[cfg(any(feature = "dump-waveform", feature = "profiling-audio-draw",
          feature = "profiling-audio-buffer", feature = "profiling-waveform-buffer",
          feature = "profile-waveform-image"))]
extern crate chrono;

use clap::{Arg, App};

use gettextrs::*;

use gtk::{Builder, BuilderExt};

mod metadata;
mod media;
mod ui;
use ui::MainController;

fn main() {
    setlocale(LocaleCategory::LcAll, "fr_FR.UTF-8");
    bindtextdomain("media-toc", "/usr/local/share/locale/");
    textdomain("media-toc");

    let matches = App::new("media-toc")
        .version("0.3.0.1")
        .author("François Laignel <fengalin@free.fr>")
        .about(concat!(
            "Build a table of contents from a media file ",
            "or split a media file into chapters",
        ))
        .arg(Arg::with_name("INPUT")
            .help("Path to the input media file")
            .index(1))
        .get_matches();

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
        builder.set_translation_domain("media-toc");
        println!("{:?}", builder.get_translation_domain());
        MainController::new(&builder)
    };
    main_ctrl.borrow().show_all();

    if let Some(input_file) = matches.value_of("INPUT") {
        main_ctrl
            .borrow_mut()
            .open_media(input_file.into());
    }

    gtk::main();
}

#[cfg(test)]
mod tests {
    use gettextrs::*;

    #[test]
    fn i18n() {
        setlocale(LocaleCategory::LcAll, "fr_FR.UTF-8");
        bindtextdomain("media-toc-test", "test/locale/");
        textdomain("media-toc-test");

        assert_eq!("ceci est un test", gettext("this is a test"));
    }
}
