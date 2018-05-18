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

use std::path::PathBuf;

mod media;
mod metadata;
mod ui;
use ui::{APP_ID, MainController};

fn init_locale() {
    match TextDomain::new("media-toc").prepend("target").init() {
        Ok(locale) => info!("Translation found, `setlocale` returned {:?}", locale),
        Err(TextDomainError::TranslationNotFound(lang)) => {
            warn!("Translation not found for language {}", lang)
        }
        Err(TextDomainError::InvalidLocale(locale)) => error!("Invalid locale {}", locale),
    }
}

pub struct CommandLineArguments {
    pub input_file: Option<PathBuf>,
    pub disable_gl: bool,
}

fn handle_command_line() -> CommandLineArguments {
    let about_msg =
        gettext("Build a table of contents from a media file\nor split a media file into chapters");
    let help_msg = gettext("Display this message");
    let version_msg = gettext("Print version information");

    let disable_gl_arg = "DISABLE_GL";
    let input_arg = gettext("MEDIA");

    let matches = App::new("media-toc")
        .version(env!("CARGO_PKG_VERSION"))
        .author("François Laignel <fengalin@free.fr>")
        .about(&about_msg[..])
        .help_message(&help_msg[..])
        .version_message(&version_msg[..])
        .arg(
            Arg::with_name(&disable_gl_arg[..])
                .short("d")
                .long("disable-gl")
                .help(&gettext("Disable hardware acceleration for video rendering"))
        )
        .arg(
            Arg::with_name(&input_arg[..])
                .help(&gettext("Path to the input media file"))
                .last(false),
        )
        .get_matches();

    CommandLineArguments {
        input_file: matches.value_of(input_arg.as_str())
            .map(|input_file| input_file.into()),
        disable_gl: matches.is_present(disable_gl_arg),
    }
}

fn run(is_gst_ok: bool, args: CommandLineArguments) {
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

    gtk_app.connect_activate(move |gtk_app| {
        let main_ctrl = MainController::new(gtk_app, is_gst_ok, args.disable_gl);
        main_ctrl.borrow().show_all();

        if is_gst_ok {
            if let Some(ref input_file) = args.input_file {
                // FIXME: there must be a lifetime way to avoid
                // all these duplications
                main_ctrl.borrow_mut().open_media(input_file.clone());
            }
        }
    });

    gtk_app.run(&[]);
}

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