use gio;
use gio::prelude::*;
use glib;
use gtk;

use ui::MainController;

pub const TLD: &str = "org";
pub const SLD: &str = "fengalin";
lazy_static! {
    pub static ref APP_ID: String = format!("{}.{}.{}", TLD, SLD, env!("CARGO_PKG_NAME"));
}

lazy_static! {
    pub static ref APP_PATH: String = format!("/{}/{}/{}", TLD, SLD, env!("CARGO_PKG_NAME"));
}

mod command_line;
pub use self::command_line::{handle_command_line, CommandLineArguments};

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

pub fn run(is_gst_ok: bool, args: CommandLineArguments) {
    register_resource(include_bytes!("../../target/resources/icons.gresource"));
    register_resource(include_bytes!("../../target/resources/ui.gresource"));

    let gtk_app = gtk::Application::new(&APP_ID[..], gio::ApplicationFlags::empty())
        .expect("Failed to initialize GtkApplication");

    gtk_app.connect_activate(move |gtk_app| {
        let main_ctrl = MainController::new(gtk_app, is_gst_ok, args.disable_gl);
        main_ctrl.borrow().show_all();

        if is_gst_ok {
            if let Some(ref input_file) = args.input_file {
                main_ctrl.borrow_mut().open_media(input_file);
            }
        }
    });

    gtk_app.run(&[]);
}
