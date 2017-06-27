extern crate gtk;
extern crate cairo;

use gtk::prelude::*;
use gtk::{Builder, ApplicationWindow, HeaderBar, DrawingArea, Statusbar};

use cairo::enums::{FontSlant, FontWeight};
use cairo::Context;

fn draw_something(da: &DrawingArea, cr: &cairo::Context) -> Inhibit {
    let allocation = da.get_allocation();
    cr.scale(allocation.width as f64, allocation.height as f64);

    cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
    cr.set_font_size(0.07);

    cr.move_to(0.1, 0.53);
    cr.show_text(&format!("{} place holder", da.get_name().unwrap()));

    Inhibit(false)
}

fn display_something(builder: &Builder) {
    let video_area: DrawingArea = builder.get_object("video-drawingarea").unwrap();
    video_area.connect_draw(draw_something);

    let audio_area: DrawingArea = builder.get_object("audio-drawingarea").unwrap();
    audio_area.connect_draw(draw_something);


    let status_bar: Statusbar = builder.get_object("status-bar").unwrap();
    status_bar.push(status_bar.get_context_id("dummy msg"), "Media-TOC prototype");
}

fn main() {
    if gtk::init().is_err() {
        panic!("Failed to initialize GTK.");
    }

    let builder = Builder::new_from_string(include_str!("media-toc.glade"));

    let window: ApplicationWindow = builder.get_object("application-window").unwrap();
    let header_bar: HeaderBar = builder.get_object("header-bar").unwrap();
    window.set_titlebar(&header_bar);

    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        Inhibit(false)
    });

    window.show_all();

    display_something(&builder);

    gtk::main();
}
