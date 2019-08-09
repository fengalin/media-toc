use gettextrs::gettext;
use glib;
use gtk;
use gtk::prelude::*;

use std::{
    collections::HashSet,
    path::Path,
    rc::Rc,
    sync::{Arc, RwLock},
};

use media::MediaEvent;
use metadata;
use metadata::{Format, MediaInfo};

use super::{PlaybackPipeline, UIController, UIEventSender, UIFocusContext};

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

type OverwriteResponseCb = Fn(gtk::ResponseType, &Rc<Path>);

pub struct OutputBaseController<Impl> {
    impl_: Impl,
    ui_event: UIEventSender,

    progress_bar: gtk::ProgressBar,
    pub(super) list: gtk::ListBox,
    pub(super) btn: gtk::Button,
    btn_default_label: glib::GString,

    perspective_selector: gtk::MenuButton,
    open_btn: gtk::Button,
    pub(super) page: gtk::Widget,

    pub(super) is_busy: bool,
    pub(super) media_event_handler: Option<Rc<Fn(MediaEvent) -> gtk::Continue>>,
    pub(super) progress_updater: Option<Rc<Fn() -> gtk::Continue>>,

    pub(super) overwrite_response_cb: Option<Rc<OverwriteResponseCb>>,
    overwrite_all: bool,
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
            open_btn: builder.get_object("open-btn").unwrap(),
            page,

            is_busy: false,
            media_event_handler: None,
            progress_updater: None,

            overwrite_response_cb: None,
            overwrite_all: false,
        };

        ctrl.btn.set_sensitive(false);

        ctrl
    }

    pub fn start(&mut self) {
        self.switch_to_busy();
        self.overwrite_all = false;

        match self.impl_.init() {
            ProcessingType::Sync => (),
            ProcessingType::Async(receiver) => {
                self.attach_media_event_handler(receiver);
                self.register_progress_timer();
            }
        }

        self.handle_processing_states(Ok(ProcessingState::Start));
    }

    pub fn cancel(&mut self) {
        self.impl_.cancel();
        self.handle_processing_states(Ok(ProcessingState::Cancelled));
    }

    #[allow(clippy::redundant_closure)]
    fn attach_media_event_handler(&mut self, receiver: glib::Receiver<MediaEvent>) {
        let media_event_handler = Rc::clone(self.media_event_handler.as_ref().unwrap());
        receiver.attach(None, move |event| media_event_handler(event));
    }

    pub fn handle_media_event(&mut self, event: MediaEvent) -> gtk::Continue {
        let res = self.impl_.handle_media_event(event);
        self.handle_processing_states(res)
    }

    fn register_progress_timer(&mut self) {
        let progress_updater = Rc::clone(self.progress_updater.as_ref().unwrap());
        glib::timeout_add_local(PROGRESS_TIMER_PERIOD, move || progress_updater());
    }

    pub fn update_progress(&mut self) -> gtk::Continue {
        if self.is_busy {
            self.progress_bar.set_fraction(self.impl_.report_progress());
            gtk::Continue(true)
        } else {
            self.progress_bar.set_fraction(0f64);
            gtk::Continue(false)
        }
    }

    fn ask_overwrite_question(&mut self, path: &Rc<Path>) {
        self.btn.set_sensitive(false);

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
        self.btn.set_sensitive(true);

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
        }

        self.handle_processing_states(Ok(next_state));
    }

    fn switch_to_busy(&mut self) {
        self.list.set_sensitive(false);
        self.btn.set_label(&gettext("Cancel"));

        self.perspective_selector.set_sensitive(false);
        self.open_btn.set_sensitive(false);

        self.ui_event.set_cursor_waiting();

        self.is_busy = true;
    }

    pub(super) fn switch_to_available(&mut self) {
        self.is_busy = false;

        self.progress_bar.set_fraction(0f64);
        self.list.set_sensitive(true);
        self.btn.set_label(self.btn_default_label.as_str());

        self.perspective_selector.set_sensitive(true);
        self.open_btn.set_sensitive(true);

        self.ui_event.reset_cursor();
    }

    fn handle_processing_states(
        &mut self,
        mut res: Result<ProcessingState, String>,
    ) -> gtk::Continue {
        let ret = loop {
            match res {
                Ok(ProcessingState::AllComplete(msg)) => {
                    self.ui_event.show_info(msg);
                    break gtk::Continue(false);
                }
                Ok(ProcessingState::Cancelled) => {
                    break gtk::Continue(false);
                }
                Ok(ProcessingState::ConfirmedOutputTo(path)) => {
                    res = self.impl_.process(path.as_ref());
                    if res == Ok(ProcessingState::PendingAsyncMediaEvent) {
                        // Next state handled asynchronously in media event handler
                        break gtk::Continue(true);
                    }
                }
                Ok(ProcessingState::DoneWithCurrent) => {
                    res = self.impl_.next();
                }
                Ok(ProcessingState::PendingAsyncMediaEvent) => {
                    // Next state handled asynchronously in media event handler
                    break gtk::Continue(true);
                }
                Ok(ProcessingState::Start) => {
                    res = self.impl_.next();
                }
                Ok(ProcessingState::SkipCurrent) => {
                    res = match self.impl_.next() {
                        Ok(state) => match state {
                            ProcessingState::AllComplete(_) => {
                                // Don't display the success message when the user decided
                                // to skip (not overwrite) last part as it seems missleading
                                break gtk::Continue(false);
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
                        break gtk::Continue(true);
                    } else {
                        // handle processing in next iteration
                        res = Ok(ProcessingState::ConfirmedOutputTo(path));
                    }
                }
                Err(err) => {
                    self.ui_event.show_error(err);
                    break gtk::Continue(false);
                }
            }
        };

        if let gtk::Continue(false) = ret {
            self.switch_to_available();
        }

        ret
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
