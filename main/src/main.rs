use gettextrs::gettext;
use log::error;

use application::{command_line, init_locale};

fn main() {
    env_logger::init();

    init_locale();

    // Character encoding is broken unless gtk (glib) is initialized
    let is_gtk_ok = gtk::init().is_ok();

    if is_gtk_ok {
        ui::run(command_line());
    } else {
        error!("{}", gettext("Failed to initialize GTK"));
    }
}
