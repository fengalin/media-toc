use gettextrs::gettext;
use log::error;

mod application;
use crate::application::{init_locale, run};
mod media;
mod metadata;
mod renderers;
mod ui;

fn main() {
    env_logger::init();

    init_locale();

    // Character encoding is broken unless gtk (glib) is initialized
    let is_gtk_ok = gtk::init().is_ok();

    if is_gtk_ok {
        run();
    } else {
        error!("{}", gettext("Failed to initialize GTK"));
    }
}
