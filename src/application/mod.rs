use gio;
use gio::prelude::*;
use glib;
use gtk;

use ui::MainController;

pub const TLD: &str = "org";
pub const SLD: &str = "fengalin";
lazy_static! {
    pub static ref APP_ID: String = TLD.to_owned() + "." + SLD + "." + env!("CARGO_PKG_NAME");
}

mod command_line;
pub use self::command_line::{CommandLineArguments, handle_command_line};

mod configuration;
pub use self::configuration::Config;

mod locale;
pub use self::locale::init_locale;

pub fn run(is_gst_ok: bool, args: CommandLineArguments) {
    // Init resources
    let res_bytes = include_bytes!("../../target/resources/icons.gresource");
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
    let gtk_app = gtk::Application::new(&APP_ID[..], gio::ApplicationFlags::empty())
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
