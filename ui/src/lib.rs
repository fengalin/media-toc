mod audio;
mod export;
mod generic_output;
mod info;
mod info_bar;
mod main;
mod perspective;
mod playback;
mod split;
mod streams;
mod video;

mod ui_event;
pub use self::ui_event::{UIEvent, UIEventChannel};

use futures::{
    channel::mpsc as async_mpsc,
    future::{self, LocalBoxFuture},
    prelude::*,
};

use gio::prelude::*;
use log::warn;

use std::{
    future::Future,
    ops::{Deref, DerefMut},
    path::Path,
    sync::{Arc, Mutex},
};

use application::{CommandLineArguments, APP_ID};

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

    let gtk_app = gtk::Application::new(Some(&APP_ID), gio::ApplicationFlags::empty())
        .expect("Failed to initialize GtkApplication");

    gtk_app.connect_activate(move |gtk_app| main::Dispatcher::setup(gtk_app, &args));
    gtk_app.run(&[]);
}

pub struct PlaybackPipeline(media::PlaybackPipeline<renderers::WaveformRenderer>);

impl PlaybackPipeline {
    pub fn try_new(
        path: &Path,
        dbl_audio_buffer_mtx: &Arc<Mutex<media::DoubleAudioBuffer<renderers::WaveformRenderer>>>,
        video_sink: &Option<gst::Element>,
        sender: async_mpsc::Sender<media::MediaEvent>,
    ) -> Result<Self, String> {
        media::PlaybackPipeline::<renderers::WaveformRenderer>::try_new(
            path,
            dbl_audio_buffer_mtx,
            video_sink,
            sender,
        )
        .map(PlaybackPipeline)
    }

    pub fn check_requirements() -> Result<(), String> {
        media::PlaybackPipeline::<renderers::WaveformRenderer>::check_requirements()
    }
}

impl Deref for PlaybackPipeline {
    type Target = media::PlaybackPipeline<renderers::WaveformRenderer>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for PlaybackPipeline {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
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
    fn new_media(&mut self, _pipeline: &PlaybackPipeline) {}
    fn cleanup(&mut self);
    fn streams_changed(&mut self, _info: &metadata::MediaInfo) {}
    fn grab_focus(&self) {}
}

pub trait UIDispatcher {
    type Controller: UIController;
    type Event;

    fn setup(ctrl: &mut Self::Controller, app: &gtk::Application);
    fn handle_event(
        _main_ctrl: &mut main::Controller,
        _event: impl Into<Self::Event>,
    ) -> LocalBoxFuture<'_, ()> {
        future::ready(()).boxed_local()
    }
    fn bind_accels_for(_ctx: UIFocusContext, _app: &gtk::Application) {}
}

pub mod prelude {
    pub use super::{PlaybackPipeline, UIController, UIDispatcher, UIEventChannel, UIFocusContext};
}
