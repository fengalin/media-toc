extern crate byteorder;
extern crate cairo;
extern crate clap;
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

use gio::prelude::*;
use gtk::Builder;

use std::path::PathBuf;

mod media;
mod metadata;
mod ui;
use ui::MainController;

const APP_ID: &str = "org.fengalin.media-toc";

fn init_locale() {
    match TextDomain::new("media-toc").prepend("target").init() {
        Ok(locale) => info!("Translation found, `setlocale` returned {:?}", locale),
        Err(TextDomainError::TranslationNotFound(lang)) => {
            warn!("Translation not found for language {}", lang)
        }
        Err(TextDomainError::InvalidLocale(locale)) => error!("Invalid locale {}", locale),
    }
}

fn handle_command_line() -> Option<PathBuf> {
    let about_msg =
        gettext("Build a table of contents from a media file\nor split a media file into chapters");
    let help_msg = gettext("Display this message");
    let version_msg = gettext("Print version information");

    let input_arg = gettext("MEDIA");

    App::new("media-toc")
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
        .get_matches()
        .value_of(input_arg.as_str())
        .map(|input_file| input_file.into())
}

fn run(is_gst_ok: bool, input_file: Option<PathBuf>) {
    // Init resources
    let res_bytes = include_bytes!("../target/resources/icons.gresource");
    let gbytes = glib::Bytes::from(res_bytes.as_ref());
    let _res = gio::Resource::new_from_data(&gbytes)
        .and_then(|resource| {
            gio::resources_register(&resource);
            Ok(())
        })
        .unwrap_or_else(|err| {
            warn!("unable to load resources: {:?}", err);
        });

    // Init App
    let gtk_app = gtk::Application::new(APP_ID, gio::ApplicationFlags::empty())
        .expect("Failed to initialize GtkApplication");

    gtk_app.connect_startup(move |gtk_app| {
        let main_ctrl = MainController::new(
            gtk_app,
            &Builder::new_from_string(include_str!("ui/media-toc.ui")),
            is_gst_ok,
        );

        let input_file = input_file.clone();
        gtk_app.connect_activate(move |_| {
            let mut main_ctrl = main_ctrl.borrow_mut();
            main_ctrl.show_all();

            if is_gst_ok {
                if let Some(ref input_file) = input_file {
                    // FIXME: there must be a lifetime way to avoid
                    // all these duplications
                    main_ctrl.open_media(input_file.clone());
                }
            }
        });
    });

    gtk_app.run(&[]);
}

fn main() {
    env_logger::init();

    init_locale();

    // Messages are not translated unless gtk (glib) is initialized
    let is_gtk_ok = gtk::init().is_ok();

    let input_file = handle_command_line();

    if is_gtk_ok {
        run(gstreamer::init().is_ok(), input_file);
    } else {
        error!("{}", gettext("Failed to initialize GTK"));
    }
}