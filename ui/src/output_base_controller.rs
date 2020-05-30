use futures::channel::mpsc as async_mpsc;
use futures::future::{abortable, AbortHandle, LocalBoxFuture};
use futures::prelude::*;

use gettextrs::gettext;
use gtk::prelude::*;

use std::{
    collections::HashSet,
    path::Path,
    rc::Rc,
    sync::{Arc, RwLock},
};

use media::MediaEvent;
use metadata::{Format, MediaInfo};

use crate::spawn;

use super::{MediaEventReceiver, PlaybackPipeline, UIController, UIEventSender, UIFocusContext};

pub const MEDIA_EVENT_CHANNEL_CAPACITY: usize = 1;

pub enum ProcessingType {
    Async(async_mpsc::Receiver<MediaEvent>),
    Sync,
}

#[derive(Debug, PartialEq)]
pub enum ProcessingState {
    AllComplete(String),
    ConfirmedOutputTo(Rc<Path>),
    SkipCurrent,
    DoneWithCurrent,
    PendingAsyncMediaEvent,
    Start,
    WouldOutputTo(Rc<Path>),
}

pub trait MediaProcessor {
    fn init(&mut self) -> ProcessingType;
    fn next(&mut self) -> Result<ProcessingState, String>;
    fn process(&mut self, path: &Path) -> Result<ProcessingState, String>;
    fn cancel(&mut self);
    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingState, String>;
    fn report_progress(&mut self) -> f64;
}

pub trait OutputControllerImpl: MediaProcessor + UIController {
    const FOCUS_CONTEXT: UIFocusContext;
    const BTN_NAME: &'static str;
    const LIST_NAME: &'static str;
    const PROGRESS_BAR_NAME: &'static str;
}

pub struct OutputMediaFileInfo {
    pub format: Format,
    pub path: Rc<Path>,
    pub extension: String,
    pub stream_ids: Arc<RwLock<HashSet<String>>>,
}

impl OutputMediaFileInfo {
    pub fn new(format: Format, src_info: &MediaInfo) -> Self {
        let (stream_ids, content) = src_info.streams.get_ids_to_export(format);
        let extension = metadata::Factory::get_extension(format, content).to_owned();

        OutputMediaFileInfo {
            path: src_info.path.with_extension(&extension).into(),
            extension,
            format,
            stream_ids: Arc::new(RwLock::new(stream_ids)),
        }
    }
}

type ProcessingStateResult = Result<ProcessingState, String>;

pub struct OutputBaseController<Impl> {
    pub(super) impl_: Impl,
    ui_event: UIEventSender,

    progress_bar: gtk::ProgressBar,
    pub(super) list: gtk::ListBox,
    pub(super) btn: gtk::Button,
    btn_default_label: glib::GString,

    perspective_selector: gtk::MenuButton,
    pub(super) open_action: Option<gio::SimpleAction>,
    pub(super) page: gtk::Widget,

    pub(super) is_busy: bool,
    pub(super) new_media_event_handler:
        Option<Box<dyn Fn(MediaEventReceiver) -> LocalBoxFuture<'static, ()>>>,
    media_event_abort_handle: Option<AbortHandle>,
    pub(super) new_progress_updater: Option<Box<dyn Fn() -> LocalBoxFuture<'static, ()>>>,

    #[allow(clippy::type_complexity)]
    pub(super) new_processing_state_handler:
        Option<Box<dyn Fn(ProcessingStateResult) -> LocalBoxFuture<'static, Result<(), ()>>>>,

    pub(super) overwrite_all: bool,
}

impl<Impl: OutputControllerImpl> OutputBaseController<Impl> {
    pub fn new_base(impl_: Impl, builder: &gtk::Builder, ui_event: UIEventSender) -> Self {
        let btn: gtk::Button = builder.get_object(Impl::BTN_NAME).unwrap();
        let list: gtk::ListBox = builder.get_object(Impl::LIST_NAME).unwrap();
        let page: gtk::Widget = list
            .get_parent()
            .unwrap_or_else(|| panic!("Couldn't get parent for list {}", Impl::LIST_NAME));

        let ctrl = OutputBaseController {
            impl_,
            ui_event,

            btn_default_label: btn.get_label().unwrap(),
            btn,
            list,
            progress_bar: builder.get_object(Impl::PROGRESS_BAR_NAME).unwrap(),

            perspective_selector: builder.get_object("perspective-menu-btn").unwrap(),
            open_action: None,
            page,

            is_busy: false,
            new_media_event_handler: None,
            media_event_abort_handle: None,
            new_progress_updater: None,

            new_processing_state_handler: None,

            overwrite_all: false,
        };

        ctrl.btn.set_sensitive(false);

        ctrl
    }

    pub fn start(&mut self) {
        self.switch_to_busy();
        self.overwrite_all = false;

        // FIXME spawn the media event handler and progress timer
        // from the implementation when necessary
        match self.impl_.init() {
            ProcessingType::Sync => (),
            ProcessingType::Async(receiver) => {
                let (abortable_handler, abort_handle) =
                    abortable(self.new_media_event_handler.as_ref().unwrap()(receiver));
                self.media_event_abort_handle = Some(abort_handle);
                spawn!(abortable_handler.map(drop));

                spawn!(self.new_progress_updater.as_ref().unwrap()());
            }
        }

        spawn!(
            self.new_processing_state_handler.as_ref().unwrap()(Ok(ProcessingState::Start))
                .map(drop)
        );
    }

    pub fn cancel(&mut self) {
        self.impl_.cancel();
        if let Some(abortable_handler) = self.media_event_abort_handle.take() {
            abortable_handler.abort();
        }
        self.switch_to_available();
    }

    pub fn update_progress(&mut self) -> Result<(), ()> {
        if self.is_busy {
            self.progress_bar.set_fraction(self.impl_.report_progress());
            Ok(())
        } else {
            self.progress_bar.set_fraction(0f64);
            Err(())
        }
    }

    fn switch_to_busy(&mut self) {
        self.list.set_sensitive(false);
        self.btn.set_label(&gettext("Cancel"));

        self.perspective_selector.set_sensitive(false);
        self.open_action.as_ref().unwrap().set_enabled(false);

        self.ui_event.set_cursor_waiting();

        self.is_busy = true;
    }

    pub(super) fn switch_to_available(&mut self) {
        self.is_busy = false;

        self.progress_bar.set_fraction(0f64);
        self.list.set_sensitive(true);
        self.btn.set_label(self.btn_default_label.as_str());

        self.perspective_selector.set_sensitive(true);
        self.open_action.as_ref().unwrap().set_enabled(true);

        self.ui_event.reset_cursor();
    }
}

impl<Impl: OutputControllerImpl> UIController for OutputBaseController<Impl> {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        self.btn.set_sensitive(true);
        self.impl_.new_media(pipeline);
    }

    fn cleanup(&mut self) {
        self.progress_bar.set_fraction(0f64);
        self.btn.set_sensitive(false);
        self.overwrite_all = false;
        self.impl_.cleanup();
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.impl_.streams_changed(info);
    }

    fn grab_focus(&self) {
        self.btn.grab_default();
        if let Some(selected_row) = self.list.get_selected_row() {
            selected_row.grab_focus();
        }
    }
}
