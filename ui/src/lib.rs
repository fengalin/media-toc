mod audio;
mod export;
mod generic_output;
mod info;
mod info_bar;
mod main_panel;
mod perspective;
mod playback;
mod split;
mod streams;
mod video;

mod ui_event;
pub use self::ui_event::{UIEvent, UIEventChannel};

use futures::{
    future::{self, LocalBoxFuture},
    prelude::*,
};

use gtk::{
    gio::{self, prelude::*},
    glib,
};
use log::warn;

use std::future::Future;

use application::{CommandLineArguments, APP_ID};
use media::pipeline;

pub fn spawn<Fut: Future<Output = ()> + 'static>(fut: Fut) {
    glib::MainContext::ref_thread_default().spawn_local(fut);
}

fn register_resource(resource: &[u8]) {
    let gbytes = glib::Bytes::from(resource);
    gio::Resource::from_data(&gbytes)
        .map(|resource| {
            gio::resources_register(&resource);
        })
        .unwrap_or_else(|err| {
            warn!("unable to load resources: {:?}", err);
        });
}

pub fn run(args: CommandLineArguments) {
    register_resource(include_bytes!("../../target/resources/icons.gresource"));
    register_resource(include_bytes!("../../target/resources/ui.gresource"));

    let gtk_app = gtk::Application::new(Some(&APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);

    // Command line is handle by claps before reaching here.
    gtk_app.connect_command_line(move |gtk_app, _cmd_line| {
        main_panel::Dispatcher::setup(gtk_app, &args);
        0i32
    });
    gtk_app.run();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UIFocusContext {
    ExportPage,
    InfoBar,
    PlaybackPage,
    SplitPage,
    StreamsPage,
    TextEntry,
}

pub trait UIController {
    fn new_media(&mut self, _pipeline: &pipeline::Playback) {}
    fn cleanup(&mut self);
    fn streams_changed(&mut self, _info: &metadata::MediaInfo) {}
    fn grab_focus(&self) {}
}

pub trait UIDispatcher {
    type Controller: UIController;
    type Event;

    fn setup(ctrl: &mut Self::Controller, app: &gtk::Application);
    fn handle_event(
        _main_ctrl: &mut main_panel::Controller,
        _event: impl Into<Self::Event>,
    ) -> LocalBoxFuture<'_, ()> {
        future::ready(()).boxed_local()
    }
    fn bind_accels_for(_ctx: UIFocusContext, _app: &gtk::Application) {}
}

pub mod prelude {
    pub use super::{UIController, UIDispatcher, UIEventChannel, UIFocusContext};
}
