use gettextrs::gettext;
use glib;
use gtk;
use gtk::prelude::*;

use std::{
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

type OverwriteResponseCb = Fn(gtk::ResponseType, &Rc<Path>);

pub struct OutputBaseController<Impl> {
    impl_: Impl,
    ui_event: UIEventSender,

    pub(super) progress_bar: gtk::ProgressBar,
    list: gtk::ListBox,
    pub(super) btn: gtk::Button,

    perspective_selector: gtk::MenuButton,
    open_btn: gtk::Button,
    chapter_grid: gtk::Grid,

    pub(super) playback_pipeline: Option<PlaybackPipeline>,

    pub(super) media_event_handler: Option<Rc<Fn(MediaEvent)>>,
    pub(super) media_event_handler_src: Option<glib::SourceId>,

    pub(super) progress_updater: Option<Rc<Fn()>>,
    pub(super) progress_timer_src: Option<glib::SourceId>,

    pub(super) overwrite_response_cb: Option<Rc<OverwriteResponseCb>>,
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

    fn register_progress_timer(&mut self) {
        debug_assert!(self.progress_timer_src.is_none());

        let progress_updater = Rc::clone(self.progress_updater.as_ref().unwrap());
        self.progress_timer_src = Some(glib::timeout_add_local(PROGRESS_TIMER_PERIOD, move || {
            progress_updater();
            glib::Continue(true)
        }));
    }

    fn remove_progress_timer(&mut self) {
        if let Some(src) = self.progress_timer_src.take() {
            let _res = glib::Source::remove(src);
        }
    }

    fn ask_overwrite_question(&self, path: &Rc<Path>) {
        self.ui_event.reset_cursor();

        let question = gettext("{output_file}\nalready exists. Overwrite?").replacen(
            "{output_file}",
            path.to_str().as_ref().unwrap(),
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
        self.handle_processing_states(Ok(ProcessingState::GotUserResponse(
            response_type,
            Rc::clone(path),
        )));
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
                    res = match self.process(path.as_ref()) {
                        Ok(()) => {
                            if self.media_event_handler_src.is_some() {
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
                    res = Ok(match response_type {
                        gtk::ResponseType::Yes => {
                            self.ui_event.set_cursor_waiting();
                            ProcessingState::ConfirmedOutputTo(path)
                        }
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
                            self.attach_media_event_handler(receiver);
                            self.register_progress_timer();
                        }
                    }

                    res = self.next();
                }
                Ok(ProcessingState::WouldOutputTo(path)) => {
                    if path.exists() {
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
