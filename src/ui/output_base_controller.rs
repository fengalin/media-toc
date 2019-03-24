use gettextrs::gettext;
use glib;
use gtk;
use gtk::prelude::*;

use std::{
    borrow::Cow,
    collections::HashSet,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, RwLock},
};

use crate::{
    media::{MediaEvent, PlaybackPipeline},
    metadata,
    metadata::{Format, MediaInfo},
};

use super::UIController;

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
    CurrentSkipped,
    DoneWithCurrent,
    GotUserResponse(gtk::ResponseType, Rc<Path>),
    InProgress,
    Start,
    WouldOutputTo(Rc<Path>),
}

pub trait MediaProcessor {
    fn init(&mut self) -> ProcessingType;
    fn next(&mut self) -> Result<ProcessingState, String>;
    fn process(&mut self, path: &Path) -> Result<(), String>;
    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingState, String>;
    fn report_progress(&mut self) -> Option<f64>;
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

    pub(super) handle_media_event_async: Option<Rc<Fn(MediaEvent) -> glib::Continue>>,
    pub(super) handle_media_event_async_src: Option<glib::SourceId>,

    pub(super) progress_updater: Option<Rc<Fn() -> glib::Continue>>,
    pub(super) progress_timer_src: Option<glib::SourceId>,

    pub(super) cursor_waiting_dispatcher: Option<Box<Fn()>>,
    pub(super) hand_back_to_main_ctrl_dispatcher: Option<Box<Fn()>>,
    pub(super) overwrite_question_dispatcher: Option<Box<Fn(String, Rc<Path>)>>,
    pub(super) show_error_dispatcher: Option<Box<Fn(Cow<'static, str>)>>,
    pub(super) show_info_dispatcher: Option<Box<Fn(Cow<'static, str>)>>,
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

            handle_media_event_async: None,
            handle_media_event_async_src: None,

            progress_updater: None,
            progress_timer_src: None,

            cursor_waiting_dispatcher: None,
            hand_back_to_main_ctrl_dispatcher: None,
            overwrite_question_dispatcher: None,
            show_error_dispatcher: None,
            show_info_dispatcher: None,
        }
    }

    #[allow(clippy::redundant_closure)]
    fn attach_handle_media_event_async(&mut self, receiver: glib::Receiver<MediaEvent>) {
        debug_assert!(self.handle_media_event_async_src.is_none());

        let handle_media_event_async = Rc::clone(
            self.handle_media_event_async
                .as_ref()
                .expect("OutputBaseController: handle_media_event_async is not defined"),
        );

        self.handle_media_event_async_src =
            Some(receiver.attach(None, move |event| handle_media_event_async(event)));
    }

    fn remove_handle_media_event_async_timer(&mut self) {
        if let Some(src) = self.handle_media_event_async_src.take() {
            let _res = glib::Source::remove(src);
        }
    }

    fn register_progress_timer(&mut self) {
        debug_assert!(self.progress_timer_src.is_none());

        let progress_updater = Rc::clone(
            self.progress_updater
                .as_ref()
                .expect("OutputBaseController: progress_updater is not defined"),
        );

        self.progress_timer_src = Some(glib::timeout_add_local(PROGRESS_TIMER_PERIOD, move || {
            progress_updater()
        }));
    }

    fn remove_progress_timer(&mut self) {
        if let Some(src) = self.progress_timer_src.take() {
            let _res = glib::Source::remove(src);
        }
    }

    fn dispatch_overwrite_question(&self, path: Rc<Path>) {
        let overwrite_question_dispatcher = self
            .overwrite_question_dispatcher
            .as_ref()
            .expect("OutputBasController: `overwrite_question_dispatcher` not defined");

        overwrite_question_dispatcher(
            gettext("{output_file}\nalready exists. Overwrite?")
                .replacen("{output_file}", path.to_str().as_ref().unwrap(), 1)
                .into(),
            path,
        );
    }

    fn show_error<Msg>(&self, msg: Msg)
    where
        Msg: Into<Cow<'static, str>>,
    {
        let show_error_dispatcher = self
            .show_error_dispatcher
            .as_ref()
            .expect("OutputBasController: `show_error_dispatcher` not defined");
        let msg = msg.into();
        show_error_dispatcher(msg);
    }

    fn show_info<Msg>(&self, msg: Msg)
    where
        Msg: Into<Cow<'static, str>>,
    {
        let show_info_dispatcher = self
            .show_info_dispatcher
            .as_ref()
            .expect("OutputBasController: `show_info_dispatcher` not defined");
        let msg = msg.into();
        show_info_dispatcher(msg);
    }

    fn set_cursor_waiting(&self) {
        let cursor_waiting_dispatcher = self
            .cursor_waiting_dispatcher
            .as_ref()
            .expect("OutputBaseController: cursor_waiting_dispatcher is not defined");
        cursor_waiting_dispatcher();
    }

    fn switch_to_busy(&self) {
        self.list.set_sensitive(false);
        self.btn.set_sensitive(false);

        self.perspective_selector.set_sensitive(false);
        self.open_btn.set_sensitive(false);
        self.chapter_grid.set_sensitive(false);

        self.set_cursor_waiting();
    }

    fn switch_to_available(&mut self) {
        self.remove_handle_media_event_async_timer();
        self.remove_progress_timer();

        self.progress_bar.set_fraction(0f64);
        self.list.set_sensitive(true);
        self.btn.set_sensitive(true);

        self.perspective_selector.set_sensitive(true);
        self.open_btn.set_sensitive(true);
        self.chapter_grid.set_sensitive(true);

        let hand_back_to_main_ctrl_dispatcher = self
            .hand_back_to_main_ctrl_dispatcher
            .as_ref()
            .expect("OutputBaseController: hand_back_to_main_ctrl_dispatcher is not defined");
        hand_back_to_main_ctrl_dispatcher();
    }

    pub fn handle_processing_states(&mut self, mut res: Result<ProcessingState, String>) {
        loop {
            match res {
                Ok(ProcessingState::AllComplete(msg)) => {
                    self.switch_to_available();
                    self.show_info(msg);
                    break;
                }
                Ok(ProcessingState::Cancelled) => {
                    self.switch_to_available();
                    self.show_info(gettext("Operation cancelled"));
                    break;
                }
                Ok(ProcessingState::ConfirmedOutputTo(path)) => {
                    res = match self.process(path.as_ref()) {
                        Ok(()) => {
                            if self.handle_media_event_async_src.is_some() {
                                // Don't handle `next()` locally if processing asynchronously
                                // Next steps handled asynchronously (media event handler)
                                break;
                            } else {
                                // processing synchronously
                                Ok(ProcessingState::DoneWithCurrent)
                            }
                        }
                        Err(err) => Err(err),
                    };
                }
                Ok(ProcessingState::CurrentSkipped) => {
                    res = match self.next() {
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
                Ok(ProcessingState::DoneWithCurrent) => {
                    res = self.next();
                }
                Ok(ProcessingState::GotUserResponse(response_type, path)) => {
                    self.set_cursor_waiting();
                    res = Ok(match response_type {
                        gtk::ResponseType::Yes => ProcessingState::ConfirmedOutputTo(path),
                        gtk::ResponseType::No => ProcessingState::CurrentSkipped,
                        gtk::ResponseType::Cancel => ProcessingState::Cancelled,
                        other => unimplemented!(
                            concat!(
                                "Response type {:?} in ",
                                "OutputBaseController::handle_processing_states (`GotUserResponse`)",
                            ),
                            other,
                        ),
                    });
                }
                Ok(ProcessingState::InProgress) => {
                    // Next steps handled asynchronously (media event handler)
                    break;
                }
                Ok(ProcessingState::Start) => {
                    self.switch_to_busy();

                    match self.init() {
                        ProcessingType::Sync => (),
                        ProcessingType::Async(receiver) => {
                            self.attach_handle_media_event_async(receiver);
                            self.register_progress_timer();
                        }
                    }

                    res = self.next();
                }
                Ok(ProcessingState::WouldOutputTo(path)) => {
                    if path.exists() {
                        self.dispatch_overwrite_question(path);

                        // Pending user confirmation
                        // Next steps handled asynchronously (see closure above)
                        break;
                    } else {
                        // handle processing in next iteration
                        res = Ok(ProcessingState::ConfirmedOutputTo(path));
                    }
                }
                Err(err) => {
                    self.switch_to_available();
                    self.show_error(err);
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
        self.impl_.cleanup();
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.impl_.streams_changed(info);
    }
}

impl<Impl> Deref for OutputBaseController<Impl>
where
    Impl: OutputControllerImpl + MediaProcessor + UIController + 'static,
{
    type Target = MediaProcessor;

    fn deref(&self) -> &Self::Target {
        &self.impl_
    }
}

impl<Impl> DerefMut for OutputBaseController<Impl>
where
    Impl: OutputControllerImpl + MediaProcessor + UIController + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.impl_
    }
}
