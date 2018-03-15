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
extern crate locale_config;
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

use locale_config::Locale;

mod metadata;
mod media;
mod ui;
use ui::MainController;

fn main() {
    let locale = Locale::current();
    setlocale(LocaleCategory::LcAll, locale.as_ref());

    // FIXME: determine where to find translations
    bindtextdomain("media-toc", "target/locale/");
    bind_textdomain_codeset("media-toc", "UTF-8");
    textdomain("media-toc");

    // Messages are not translated unless gtk (glib) is initialized
    let is_gtk_ok = gtk::init().is_ok();

    let about_msg = gettext(
        "Build a table of contents from a media file\nor split a media file into chapters",
    );
    let help_msg = gettext("Display this message");
    let version_msg = gettext("Print version information");

    let input_arg = gettext("MEDIA");

    let matches = App::new("media-toc")
        .version("0.3.0.1")
        .author("François Laignel <fengalin@free.fr>")
        .about(about_msg.as_str())
        .help_message(help_msg.as_str())
        .version_message(version_msg.as_str())
        .arg(Arg::with_name(input_arg.as_str())
            .help(&gettext("Path to the input media file"))
            .last(false))
        .get_matches();


    if !is_gtk_ok {
        panic!(gettext("Failed to initialize GTK"));
    }
    gstreamer::init().unwrap();

    println!("Locale: {}", locale);

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

    if let Some(input_file) = matches.value_of(input_arg.as_str()) {
        main_ctrl
            .borrow_mut()
            .open_media(input_file.into());
    }

    gtk::main();
}

#[cfg(test)]
mod tests {
    use gettextrs::*;
    use locale_config::Locale;

    #[test]
    fn i18n() {
        println!("Current locale: {}", Locale::current().as_ref());
        println!("setlocale returned {:?}", setlocale(LocaleCategory::LcAll, "en_US.UTF-8"));
        bindtextdomain("media-toc-test", "test/locale/");
        bind_textdomain_codeset("media-toc-test", "UTF-8");
        textdomain("media-toc-test");

        assert_eq!("this is a test", gettext("test-msg"));
    }
}
