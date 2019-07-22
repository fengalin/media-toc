use gio;
use gio::prelude::*;
use glib;
use gtk;
use lazy_static::lazy_static;
use log::warn;

use crate::ui::MainController;

pub const TLD: &str = "org";
pub const SLD: &str = "fengalin";
lazy_static! {
    pub static ref APP_ID: String = format!("{}.{}.{}", TLD, SLD, env!("CARGO_PKG_NAME"));
}

lazy_static! {
    pub static ref APP_PATH: String = format!("/{}/{}/{}", TLD, SLD, env!("CARGO_PKG_NAME"));
}

mod command_line;
pub use self::command_line::{get_command_line, CommandLineArguments};

mod configuration;
pub use self::configuration::CONFIG;

mod locale;
pub use self::locale::init_locale;

fn register_resource(resource: &[u8]) {
    let gbytes = glib::Bytes::from(resource);
    gio::Resource::new_from_data(&gbytes)
        .and_then(|resource| {
            gio::resources_register(&resource);
            Ok(())
        })
        .unwrap_or_else(|err| {
            warn!("unable to load resources: {:?}", err);
        });
}

pub fn run() {
    let args = get_command_line();

    register_resource(include_bytes!("../../target/resources/icons.gresource"));
    register_resource(include_bytes!("../../target/resources/ui.gresource"));

    let gtk_app = gtk::Application::new(Some(&APP_ID), gio::ApplicationFlags::empty())
        .expect("Failed to initialize GtkApplication");

    gtk_app.connect_activate(move |gtk_app| MainController::setup(gtk_app, &args));
    gtk_app.run(&[]);
}
