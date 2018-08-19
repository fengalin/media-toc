use gettextrs::gettext;
use log::error;

mod application;
use crate::application::{handle_command_line, init_locale, run};
mod media;
mod metadata;
mod ui;

fn main() {
    env_logger::init();

    init_locale();

    // Character encoding is broken unless gtk (glib) is initialized
    let is_gtk_ok = gtk::init().is_ok();

    let args = handle_command_line();

    if is_gtk_ok {
        run(gstreamer::init().is_ok(), args);
    } else {
        error!("{}", gettext("Failed to initialize GTK"));
    }
}
