use gettextrs::gettext;
use glib;
use gtk;
use gtk::prelude::*;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, RwLock},
};

use crate::{
    media::{MediaEvent, PlaybackPipeline},
    metadata,
    metadata::{Format, MediaInfo},
};

use super::{UIController, UIEventSender};

const PROGRESS_TIMER_PERIOD: u32 = 250; // 250 ms

pub enum ProcessingType {
    Async(glib::Receiver<MediaEvent>),
    Sync,
}

#[derive(Debug, PartialEq)]
pub enum ProcessingState {
    AllComplete(String),
    Cancelled,
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
    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingState, String>;
    fn report_progress(&self) -> Option<f64>;
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

type OverwriteResponseCb = Fn(gtk::ResponseType, &Rc<Path>);

pub struct OutputBaseController<Impl> {
    impl_: Impl,
    ui_event: UIEventSender,

    progress_bar: gtk::ProgressBar,
    list: gtk::ListBox,
    pub(super) btn: gtk::Button,

    perspective_selector: gtk::MenuButton,
    open_btn: gtk::Button,
    chapter_grid: gtk::Grid,

    playback_pipeline: Option<PlaybackPipeline>,

    pub(super) media_event_handler: Option<Rc<Fn(MediaEvent)>>,
    media_event_handler_src: Option<glib::SourceId>,

    pub(super) progress_updater: Option<Rc<Fn()>>,
    progress_timer_src: Option<glib::SourceId>,

    pub(super) overwrite_response_cb: Option<Rc<OverwriteResponseCb>>,
    overwrite_all: bool,
}

impl<Impl> OutputBaseController<Impl>
where
    Impl: OutputControllerImpl + MediaProcessor + UIController + 'static,
{
    pub fn new_base(impl_: Impl, builder: &gtk::Builder, ui_event_sender: UIEventSender) -> Self {
        OutputBaseController {
            impl_,
            ui_event: ui_event_sender,

            btn: builder.get_object(Impl::BTN_NAME).unwrap(),
            list: builder.get_object(Impl::LIST_NAME).unwrap(),
            progress_bar: builder.get_object(Impl::PROGRESS_BAR_NAME).unwrap(),

            perspective_selector: builder.get_object("perspective-menu-btn").unwrap(),
            open_btn: builder.get_object("open-btn").unwrap(),
            chapter_grid: builder.get_object("info-chapter_list-grid").unwrap(),

            playback_pipeline: None,

            media_event_handler: None,
            media_event_handler_src: None,

            progress_updater: None,
            progress_timer_src: None,

            overwrite_response_cb: None,
            overwrite_all: false,
        }
    }

    #[allow(clippy::redundant_closure)]
    fn attach_media_event_handler(&mut self, receiver: glib::Receiver<MediaEvent>) {
        debug_assert!(self.media_event_handler_src.is_none());

        let media_event_handler = Rc::clone(self.media_event_handler.as_ref().unwrap());
        self.media_event_handler_src = Some(receiver.attach(None, move |event| {
            media_event_handler(event);
            // will be removed in `OutputBaseController::switch_to_available`
            glib::Continue(true)
        }));
    }

    fn remove_media_event_handler(&mut self) {
        if let Some(src) = self.media_event_handler_src.take() {
            let _res = glib::Source::remove(src);
        }
    }

    pub fn handle_media_event(&mut self, event: MediaEvent) {
        let res = self.impl_.handle_media_event(event);
        self.handle_processing_states(res);
    }

    fn register_progress_timer(&mut self) {
        if self.progress_timer_src.is_none() {
            let progress_updater = Rc::clone(self.progress_updater.as_ref().unwrap());
            self.progress_timer_src =
                Some(glib::timeout_add_local(PROGRESS_TIMER_PERIOD, move || {
                    progress_updater();
                    glib::Continue(true)
                }));
        }
    }

    fn remove_progress_timer(&mut self) {
        if let Some(src) = self.progress_timer_src.take() {
            let _res = glib::Source::remove(src);
        }
    }

    pub fn update_progress(&self) {
        if let Some(progress) = self.impl_.report_progress() {
            self.progress_bar.set_fraction(progress);
        }
    }

    pub fn have_pipeline(&mut self, playback_pipeline: PlaybackPipeline) {
        self.playback_pipeline = Some(playback_pipeline);
    }

    fn ask_overwrite_question(&mut self, path: &Rc<Path>) {
        self.remove_progress_timer();
        self.ui_event.reset_cursor();

        let filename = path.file_name().expect("no `filename` in `path`");
        let filename = filename
            .to_str()
            .expect("can't get printable `str` from `filename`");
        let question = gettext("{output_file}\nalready exists. Overwrite?").replacen(
            "{output_file}",
            filename,
            1,
        );

        let path_cb = Rc::clone(path);
        let overwrite_response_cb = Rc::clone(self.overwrite_response_cb.as_ref().unwrap());
        self.ui_event.ask_question(
            question,
            Rc::new(move |response_type| overwrite_response_cb(response_type, &path_cb)),
        );
    }

    pub fn handle_overwrite_response(&mut self, response_type: gtk::ResponseType, path: &Rc<Path>) {
        let next_state = match response_type {
            gtk::ResponseType::Apply => {
                // This one is used for "Yes to all"
                self.overwrite_all = true;
                ProcessingState::ConfirmedOutputTo(Rc::clone(path))
            }
            gtk::ResponseType::Cancel => ProcessingState::Cancelled,
            gtk::ResponseType::No => ProcessingState::SkipCurrent,
            gtk::ResponseType::Yes => ProcessingState::ConfirmedOutputTo(Rc::clone(path)),
            other => unimplemented!(
                "Response type {:?} in OutputBaseController::handle_overwrite_response",
                other,
            ),
        };

        if next_state != ProcessingState::Cancelled {
            self.ui_event.set_cursor_waiting();
            self.register_progress_timer();
        }

        self.handle_processing_states(Ok(next_state));
    }

    fn switch_to_busy(&self) {
        self.list.set_sensitive(false);
        self.btn.set_sensitive(false);

        self.perspective_selector.set_sensitive(false);
        self.open_btn.set_sensitive(false);
        self.chapter_grid.set_sensitive(false);

        self.ui_event.set_cursor_waiting();
    }

    fn switch_to_available(&mut self) {
        self.remove_media_event_handler();
        self.remove_progress_timer();

        self.progress_bar.set_fraction(0f64);
        self.list.set_sensitive(true);
        self.btn.set_sensitive(true);

        self.perspective_selector.set_sensitive(true);
        self.open_btn.set_sensitive(true);
        self.chapter_grid.set_sensitive(true);

        let playback_pipeline = self.playback_pipeline.take().expect(concat!(
            "OutputBaseController: `playback_pipeline` is already taken in ",
            "`switch_to_available`",
        ));
        self.ui_event.hand_back_pipeline(playback_pipeline);
        self.ui_event.reset_cursor();
    }

    pub fn handle_processing_states(&mut self, mut res: Result<ProcessingState, String>) {
        loop {
            match res {
                Ok(ProcessingState::AllComplete(msg)) => {
                    self.switch_to_available();
                    self.ui_event.show_info(msg);
                    break;
                }
                Ok(ProcessingState::Cancelled) => {
                    self.switch_to_available();
                    self.ui_event.show_info(gettext("Operation cancelled"));
                    break;
                }
                Ok(ProcessingState::ConfirmedOutputTo(path)) => {
                    res = self.impl_.process(path.as_ref());
                    if res == Ok(ProcessingState::PendingAsyncMediaEvent) {
                        // Next state handled asynchronously in media event handler
                        break;
                    }
                }
                Ok(ProcessingState::DoneWithCurrent) => {
                    res = self.impl_.next();
                }
                Ok(ProcessingState::PendingAsyncMediaEvent) => {
                    // Next state handled asynchronously in media event handler
                    break;
                }
                Ok(ProcessingState::Start) => {
                    self.switch_to_busy();
                    self.overwrite_all = false;

                    match self.impl_.init() {
                        ProcessingType::Sync => (),
                        ProcessingType::Async(receiver) => {
                            self.attach_media_event_handler(receiver);
                            self.register_progress_timer();
                        }
                    }

                    res = self.impl_.next();
                }
                Ok(ProcessingState::SkipCurrent) => {
                    res = match self.impl_.next() {
                        Ok(state) => match state {
                            ProcessingState::AllComplete(_) => {
                                // Don't display the success message when the user decided
                                // to skip (not overwrite) last part as it seems missleading
                                self.switch_to_available();
                                break;
                            }
                            other => Ok(other),
                        },
                        Err(err) => Err(err),
                    };
                }
                Ok(ProcessingState::WouldOutputTo(path)) => {
                    if !self.overwrite_all && path.exists() {
                        self.ask_overwrite_question(&path);
                        // Pending user response
                        // Next steps handled asynchronously (see closure above)
                        break;
                    } else {
                        // handle processing in next iteration
                        res = Ok(ProcessingState::ConfirmedOutputTo(path));
                    }
                }
                Err(err) => {
                    self.switch_to_available();
                    self.ui_event.show_error(err);
                    break;
                }
            }
        }
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
        self.overwrite_all = false;
        self.impl_.cleanup();
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.impl_.streams_changed(info);
    }
}
