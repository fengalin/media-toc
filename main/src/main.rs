use gettextrs::gettext;
use log::error;

use application::{get_command_line, init_locale};

fn main() {
    env_logger::init();

    init_locale();

    // Character encoding is broken unless gtk (glib) is initialized
    let is_gtk_ok = gtk::init().is_ok();

    if is_gtk_ok {
        ui::run(get_command_line());
    } else {
        error!("{}", gettext("Failed to initialize GTK"));
    }
}
