extern crate gtk;
extern crate cairo;

use gtk::Builder;

mod main_controller;
use main_controller::MainController;

mod video_controller;
mod audio_controller;


fn main() {
    if gtk::init().is_err() {
        panic!("Failed to initialize GTK.");
    }

    let builder = Builder::new_from_string(include_str!("media-toc.ui"));
    let main_ctrl = MainController::new(builder);
    main_ctrl.borrow().show_all();

    gtk::main();
}
