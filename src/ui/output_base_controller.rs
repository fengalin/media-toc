use glib;

use gtk;
use gtk::prelude::*;

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use crate::{
    media::{MediaEvent, PlaybackPipeline},
    metadata,
    metadata::{Format, MediaInfo},
};

use super::UIController;

pub enum ProcessingType {
    Async(glib::Receiver<MediaEvent>),
    Sync,
}

pub enum ProcessingStatus {
    Completed(String),
    InProgress,
}

pub trait MediaProcessor {
    fn init(&mut self) -> ProcessingType;
    fn start(&mut self) -> Result<ProcessingStatus, String>;
    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingStatus, String>;
    fn report_progress(&mut self) -> f64;
}

pub trait OutputControllerImpl {
    const BTN_NAME: &'static str;
    const LIST_NAME: &'static str;
    const PROGRESS_BAR_NAME: &'static str;
}

pub struct OutputMediaFileInfo {
    pub format: Format,
    pub path: PathBuf,
    pub extension: String,
    pub stream_ids: Arc<RwLock<HashSet<String>>>,
}

impl OutputMediaFileInfo {
    pub fn new(format: Format, src_info: &MediaInfo) -> Self {
        let (stream_ids, content) = src_info.get_stream_ids_to_export(format);
        let extension = metadata::Factory::get_extension(format, content).to_owned();

        OutputMediaFileInfo {
            path: src_info.path.with_extension(&extension),
            extension,
            format,
            stream_ids: Arc::new(RwLock::new(stream_ids)),
        }
    }
}

pub struct OutputBaseController<Impl> {
    impl_: Impl,

    pub(super) progress_bar: gtk::ProgressBar,
    list: gtk::ListBox,
    pub(super) btn: gtk::Button,

    perspective_selector: gtk::MenuButton,
    open_btn: gtk::Button,
    chapter_grid: gtk::Grid,

    pub(super) playback_pipeline: Option<PlaybackPipeline>,

    pub(super) progress_timer_src: Option<glib::SourceId>,
}

impl<Impl> OutputBaseController<Impl>
where
    Impl: OutputControllerImpl + MediaProcessor + UIController + 'static,
{
    pub fn new_base(impl_: Impl, builder: &gtk::Builder) -> Self {
        OutputBaseController {
            impl_,

            btn: builder.get_object(Impl::BTN_NAME).unwrap(),
            list: builder.get_object(Impl::LIST_NAME).unwrap(),
            progress_bar: builder.get_object(Impl::PROGRESS_BAR_NAME).unwrap(),

            perspective_selector: builder.get_object("perspective-menu-btn").unwrap(),
            open_btn: builder.get_object("open-btn").unwrap(),
            chapter_grid: builder.get_object("info-chapter_list-grid").unwrap(),

            playback_pipeline: None,

            progress_timer_src: None,
        }
    }

    pub fn set_progress_timer_src(&mut self, src: glib::SourceId) {
        debug_assert!(self.progress_timer_src.is_none());
        self.progress_timer_src = Some(src);
    }

    fn remove_progress_timer(&mut self) {
        if let Some(src_id) = self.progress_timer_src.take() {
            let _res = glib::Source::remove(src_id);
        }
    }

    pub fn switch_to_busy(&self) {
        self.list.set_sensitive(false);
        self.btn.set_sensitive(false);

        self.perspective_selector.set_sensitive(false);
        self.open_btn.set_sensitive(false);
        self.chapter_grid.set_sensitive(false);
    }

    pub fn switch_to_available(&mut self) {
        self.remove_progress_timer();

        self.progress_bar.set_fraction(0f64);
        self.list.set_sensitive(true);
        self.btn.set_sensitive(true);

        self.perspective_selector.set_sensitive(true);
        self.open_btn.set_sensitive(true);
        self.chapter_grid.set_sensitive(true);
    }
}

impl<Impl> UIController for OutputBaseController<Impl>
where
    Impl: OutputControllerImpl + MediaProcessor + UIController + 'static,
{
    fn setup(&mut self) {
        self.btn.set_sensitive(false);
        self.impl_.setup();
    }

    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        self.btn.set_sensitive(true);
        self.impl_.new_media(pipeline);
    }

    fn cleanup(&mut self) {
        self.progress_bar.set_fraction(0f64);
        self.btn.set_sensitive(false);
        self.impl_.cleanup();
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.impl_.streams_changed(info);
    }
}

impl<Impl> MediaProcessor for OutputBaseController<Impl>
where
    Impl: OutputControllerImpl + MediaProcessor + UIController + 'static,
{
    fn init(&mut self) -> ProcessingType {
        self.impl_.init()
    }

    fn start(&mut self) -> Result<ProcessingStatus, String> {
        self.impl_.start()
    }

    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingStatus, String> {
        self.impl_.handle_media_event(event)
    }

    fn report_progress(&mut self) -> f64 {
        self.impl_.report_progress()
    }
}
