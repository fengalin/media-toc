mod audio_controller;
use self::audio_controller::{AudioAreaEvent, AudioController};
mod audio_dispatcher;
use self::audio_dispatcher::AudioDispatcher;

mod chapters_boundaries;
use self::chapters_boundaries::{ChapterTimestamps, ChaptersBoundaries};

mod chapter_tree_manager;
use self::chapter_tree_manager::{ChapterEntry, ChapterTreeManager, PositionStatus};

mod export_controller;
use self::export_controller::ExportController;
mod export_dispatcher;
use self::export_dispatcher::ExportDispatcher;

mod info_bar_controller;
use self::info_bar_controller::InfoBarController;

mod info_controller;
use self::info_controller::InfoController;
mod info_dispatcher;
use self::info_dispatcher::InfoDispatcher;

mod main_controller;
use self::main_controller::{ControllerState, MainController};
mod main_dispatcher;
use self::main_dispatcher::MainDispatcher;

mod output_base_controller;
mod output_base_dispatcher;
use self::output_base_dispatcher::OutputDispatcher;

mod perspective_controller;
use self::perspective_controller::PerspectiveController;
mod perspective_dispatcher;
use self::perspective_dispatcher::PerspectiveDispatcher;

mod streams_controller;
use self::streams_controller::{StreamClickedStatus, StreamsController};
mod streams_dispatcher;
use self::streams_dispatcher::StreamsDispatcher;

mod split_controller;
use self::split_controller::SplitController;
mod split_dispatcher;
use self::split_dispatcher::SplitDispatcher;

mod ui_event;
use self::ui_event::{UIEventSender, UIFocusContext};

mod video_controller;
use self::video_controller::VideoController;
mod video_dispatcher;
use self::video_dispatcher::VideoDispatcher;

mod waveform_with_overlay;
use self::waveform_with_overlay::WaveformWithOverlay;

use futures::channel::mpsc as async_mpsc;
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

    gtk_app.connect_activate(move |gtk_app| MainDispatcher::setup(gtk_app, &args));
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

pub trait UIController {
    fn new_media(&mut self, _pipeline: &PlaybackPipeline) {}
    fn cleanup(&mut self);
    fn streams_changed(&mut self, _info: &metadata::MediaInfo) {}
    fn grab_focus(&self) {}
}

pub trait UIDispatcher {
    type Controller: UIController;

    fn setup(ctrl: &mut Self::Controller, app: &gtk::Application, ui_event: &UIEventSender);

    // bind context specific accels
    fn bind_accels_for(_ctx: UIFocusContext, _app: &gtk::Application) {}
}
