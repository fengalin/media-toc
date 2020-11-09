use futures::{
    channel::{mpsc as async_mpsc, oneshot},
    future::{abortable, AbortHandle},
    prelude::*,
    stream,
};

use gettextrs::gettext;
use gtk::prelude::*;

use log::debug;

use std::{
    collections::HashSet,
    fmt,
    path::Path,
    rc::Rc,
    sync::{Arc, RwLock},
    time::Duration,
};

use media::MediaEvent;
use metadata::{Format, MediaInfo};

use crate::{generic_output, info_bar, main, prelude::*, spawn, UIEvent};

pub const MEDIA_EVENT_CHANNEL_CAPACITY: usize = 1;
const PROGRESS_TIMER_PERIOD: Duration = Duration::from_millis(250);

pub enum ProcessingType {
    Async(async_mpsc::Receiver<media::MediaEvent>),
    Sync,
}

pub enum MediaEventHandling {
    Done,
    ExpectingMore,
}

impl MediaEventHandling {
    pub fn is_done(&self) -> bool {
        matches!(self, MediaEventHandling::Done)
    }
}

#[derive(Debug, PartialEq)]
enum ProcessingState {
    AllDone,
    ConfirmedOutputTo(Rc<Path>),
    Init,
    SkipCurrent,
    DoneWithCurrent,
    WouldOutputTo(Rc<Path>),
}

#[derive(Debug)]
pub struct MediaProcessorError(String);

impl From<String> for MediaProcessorError {
    fn from(msg: String) -> Self {
        MediaProcessorError(msg)
    }
}

impl fmt::Display for MediaProcessorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MediaProcessorError {}

pub trait MediaProcessorImpl: Iterator<Item = Rc<Path>> {
    fn process(&mut self, output_path: &Path) -> Result<ProcessingType, MediaProcessorError>;
    fn cancel(&mut self);
    fn handle_media_event(
        &mut self,
        event: MediaEvent,
    ) -> Result<MediaEventHandling, MediaProcessorError>;
    fn report_progress(&mut self) -> f64;
    fn completion_msg() -> String;
}

struct MediaProcessor<CtrlImpl: OutputControllerImpl + 'static> {
    impl_: CtrlImpl::MediaProcessorImplType,
    progress_bar: gtk::ProgressBar,
    btn: gtk::Button,
}

impl<CtrlImpl: OutputControllerImpl + 'static> MediaProcessor<CtrlImpl> {
    pub(super) async fn spawn(
        impl_: CtrlImpl::MediaProcessorImplType,
        progress_bar: gtk::ProgressBar,
        btn: gtk::Button,
    ) -> AbortHandle {
        let mut this = MediaProcessor {
            impl_,
            progress_bar,
            btn,
        };

        let (sender, receiver) = oneshot::channel();
        spawn(async move {
            let (abortable_run, abort_handle) = abortable(Self::run(&mut this));
            sender.send(abort_handle).unwrap();

            if abortable_run.await.is_err() {
                this.impl_.cancel();
            }

            UIEventChannel::send(CtrlImpl::OutputEvent::from(
                generic_output::Event::ActionOver,
            ));
        });

        receiver.await.unwrap()
    }

    async fn run(&mut self) {
        use ProcessingState::*;

        let mut state = ProcessingState::Init;
        let mut overwrite_all = false;

        loop {
            match state {
                AllDone => {
                    info_bar::show_info(CtrlImpl::MediaProcessorImplType::completion_msg());
                    break;
                }
                ConfirmedOutputTo(path) => {
                    if let Err(err) = self.process(path.as_ref()).await {
                        info_bar::show_error(err);
                        break;
                    }

                    state = DoneWithCurrent;
                }
                Init | DoneWithCurrent => {
                    state = self.impl_.next().map_or(AllDone, WouldOutputTo);
                }
                SkipCurrent => {
                    match self.impl_.next() {
                        Some(path) => state = WouldOutputTo(path),
                        None => {
                            // Don't display the success message when the user decided
                            // to skip (not overwrite) last part as it seems missleading
                            break;
                        }
                    }
                }
                WouldOutputTo(path) => {
                    if !path.exists() || overwrite_all {
                        state = ConfirmedOutputTo(path);
                        continue;
                    }

                    // Path exists and overwrite_all is not true
                    self.btn.set_sensitive(false);
                    main::reset_cursor();

                    let filename = path
                        .file_name()
                        .expect("no `filename` in `path`")
                        .to_str()
                        .expect("can't get printable `str` from `filename`");
                    let question = gettext("{output_file}\nalready exists. Overwrite?").replacen(
                        "{output_file}",
                        filename,
                        1,
                    );

                    let response = info_bar::ask_question(question).await;
                    self.btn.set_sensitive(true);

                    let next_state = match response {
                        gtk::ResponseType::Apply => {
                            // This one is used for "Yes to all"
                            overwrite_all = true;
                            ConfirmedOutputTo(Rc::clone(&path))
                        }
                        gtk::ResponseType::Cancel => {
                            self.impl_.cancel();
                            break;
                        }
                        gtk::ResponseType::No => SkipCurrent,
                        gtk::ResponseType::Yes => ConfirmedOutputTo(Rc::clone(&path)),
                        other => unimplemented!("{:?}", other),
                    };

                    main::set_cursor_waiting();
                    state = next_state;
                }
            }
        }
    }

    async fn process(&mut self, output_path: &Path) -> Result<(), MediaProcessorError> {
        enum Item {
            MediaEvent(MediaEvent),
            Tick,
        }

        let media_event_stream = match self.impl_.process(output_path)? {
            ProcessingType::Async(media_event_stream) => media_event_stream,
            ProcessingType::Sync => return Ok(()),
        };

        // async processing

        let tick_stream = glib::interval_stream(PROGRESS_TIMER_PERIOD);
        let mut combined_streams = stream::select(
            media_event_stream.map(Item::MediaEvent),
            tick_stream.map(|_| Item::Tick),
        );

        while let Some(item) = combined_streams.next().await {
            match item {
                Item::MediaEvent(media_event) => {
                    debug!("handling media event {:?}", media_event);
                    if self.impl_.handle_media_event(media_event)?.is_done() {
                        break;
                    }
                }
                Item::Tick => {
                    self.progress_bar.set_fraction(self.impl_.report_progress());
                }
            }
        }

        Ok(())
    }
}

pub trait OutputControllerImpl: UIController {
    type MediaProcessorImplType: MediaProcessorImpl + 'static;
    type OutputEvent: From<generic_output::Event>
        + Into<generic_output::Event>
        + Into<UIEvent>
        + 'static;

    const FOCUS_CONTEXT: UIFocusContext;
    const BTN_NAME: &'static str;
    const LIST_NAME: &'static str;
    const PROGRESS_BAR_NAME: &'static str;

    fn new_processor(&self) -> Self::MediaProcessorImplType;
}

pub struct OutputMediaFileInfo {
    pub format: Format,
    pub path: Rc<Path>,
    pub extension: String,
    pub stream_ids: Arc<RwLock<HashSet<String>>>,
}

impl OutputMediaFileInfo {
    pub fn new(format: Format, src_info: &MediaInfo) -> Self {
        let (stream_ids, content) = src_info.streams.ids_to_export(format);
        let extension = metadata::Factory::extension(format, content).to_owned();

        OutputMediaFileInfo {
            path: src_info.path.with_extension(&extension).into(),
            extension,
            format,
            stream_ids: Arc::new(RwLock::new(stream_ids)),
        }
    }
}

pub struct Controller<Impl> {
    pub(super) impl_: Impl,

    progress_bar: gtk::ProgressBar,
    list: gtk::ListBox,
    pub(super) btn: gtk::Button,
    btn_default_label: glib::GString,

    perspective_selector: gtk::MenuButton,
    pub(super) open_action: Option<gio::SimpleAction>,
    pub(super) page: gtk::Widget,

    pub(super) is_busy: bool,
    processor_abort_handle: Option<AbortHandle>,
}

impl<Impl: OutputControllerImpl + 'static> Controller<Impl> {
    pub fn new_generic(impl_: Impl, builder: &gtk::Builder) -> Self {
        let btn: gtk::Button = builder.get_object(Impl::BTN_NAME).unwrap();
        btn.set_sensitive(false);
        let list: gtk::ListBox = builder.get_object(Impl::LIST_NAME).unwrap();
        let page: gtk::Widget = list
            .get_parent()
            .unwrap_or_else(|| panic!("Couldn't get parent for list {}", Impl::LIST_NAME));

        Controller {
            impl_,
            btn_default_label: btn.get_label().unwrap(),
            btn,
            list,
            progress_bar: builder.get_object(Impl::PROGRESS_BAR_NAME).unwrap(),
            perspective_selector: builder.get_object("perspective-menu-btn").unwrap(),
            open_action: None,
            page,
            is_busy: false,
            processor_abort_handle: None,
        }
    }

    pub async fn start(&mut self) {
        self.switch_to_busy();

        self.processor_abort_handle = Some(
            MediaProcessor::<Impl>::spawn(
                self.impl_.new_processor(),
                self.progress_bar.clone(),
                self.btn.clone(),
            )
            .await,
        );
    }

    pub fn cancel(&mut self) {
        if let Some(abort_handle) = self.processor_abort_handle.take() {
            abort_handle.abort();
        }
    }

    fn switch_to_busy(&mut self) {
        self.list.set_sensitive(false);
        self.btn.set_label(&gettext("Cancel"));

        self.perspective_selector.set_sensitive(false);
        self.open_action.as_ref().unwrap().set_enabled(false);

        main::set_cursor_waiting();

        self.is_busy = true;
    }

    pub(crate) fn switch_to_available(&mut self) {
        self.processor_abort_handle = None;
        self.is_busy = false;

        self.progress_bar.set_fraction(0f64);
        self.list.set_sensitive(true);
        self.btn.set_label(self.btn_default_label.as_str());

        self.perspective_selector.set_sensitive(true);
        self.open_action.as_ref().unwrap().set_enabled(true);

        main::reset_cursor();
    }
}

impl<Impl: OutputControllerImpl> UIController for Controller<Impl> {
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

    fn grab_focus(&self) {
        self.btn.grab_default();
        if let Some(selected_row) = self.list.get_selected_row() {
            selected_row.grab_focus();
        }
    }
}
