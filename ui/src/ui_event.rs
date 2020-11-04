use futures::channel::mpsc as async_mpsc;
use futures::channel::oneshot;

use std::{borrow::Cow, cell::RefCell, path::PathBuf};

use media::Timestamp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UIFocusContext {
    ExportPage,
    InfoBar,
    PlaybackPage,
    SplitPage,
    StreamsPage,
    TextEntry,
}

#[derive(Debug)]
pub(crate) enum UIEvent {
    About,
    AskQuestion {
        question: Cow<'static, str>,
        response_sender: oneshot::Sender<gtk::ResponseType>,
    },
    CancelSelectMedia,
    ChapterClicked(gtk::TreePath),
    HideInfoBar,
    NextChapter,
    OpenMedia(PathBuf),
    PlayRange {
        start: Timestamp,
        end: Timestamp,
        ts_to_restore: Timestamp,
    },
    PreviousChapter,
    Quit,
    RefreshInfo(Timestamp),
    ResetCursor,
    RestoreContext,
    Seek {
        target: Timestamp,
        flags: gst::SeekFlags,
    },
    SetCursorWaiting,
    SetCursorDoubleArrow,
    ShowAll,
    ShowError(Cow<'static, str>),
    ShowInfo(Cow<'static, str>),
    StepBack,
    StepForward,
    StreamClicked(gst::StreamType),
    StreamExportToggled(gst::StreamType, gtk::TreePath),
    SwitchTo(UIFocusContext),
    TemporarilySwitchTo(UIFocusContext),
    Tick,
    ToggleChapterList(bool),
    ToggleRepeat(bool),
    UpdateAudioRenderingCndt {
        dimensions: Option<(f64, f64)>,
    },
    UpdateFocus,
    ZoomIn,
    ZoomOut,
}

#[derive(Clone)]
pub struct UIEventSender(RefCell<async_mpsc::UnboundedSender<UIEvent>>);

#[allow(unused_must_use)]
impl UIEventSender {
    fn send(&self, event: UIEvent) {
        let _ = self.0.borrow_mut().unbounded_send(event);
    }

    pub fn about(&self) {
        self.send(UIEvent::About);
    }

    pub async fn ask_question<Q>(&self, question: Q) -> gtk::ResponseType
    where
        Q: Into<Cow<'static, str>>,
    {
        let (response_sender, response_receiver) = oneshot::channel();
        self.send(UIEvent::AskQuestion {
            question: question.into(),
            response_sender,
        });

        response_receiver.await.unwrap_or(gtk::ResponseType::Cancel)
    }

    pub fn cancel_select_media(&self) {
        self.send(UIEvent::CancelSelectMedia);
        self.reset_cursor();
    }

    pub fn chapter_clicked(&self, tree_path: gtk::TreePath) {
        self.send(UIEvent::ChapterClicked(tree_path));
    }

    pub fn hide_info_bar(&self) {
        self.send(UIEvent::HideInfoBar);
    }

    pub fn open_media(&self, path: PathBuf) {
        self.set_cursor_waiting();
        self.send(UIEvent::OpenMedia(path));
    }

    pub fn next_chapter(&self) {
        self.send(UIEvent::NextChapter);
    }

    pub fn play_range(&self, start: Timestamp, end: Timestamp, ts_to_restore: Timestamp) {
        self.send(UIEvent::PlayRange {
            start,
            end,
            ts_to_restore,
        });
    }

    pub fn previous_chapter(&self) {
        self.send(UIEvent::PreviousChapter);
    }

    pub fn quit(&self) {
        self.send(UIEvent::Quit);
    }

    pub fn refresh_info(&self, ts: Timestamp) {
        self.send(UIEvent::RefreshInfo(ts));
    }

    pub fn reset_cursor(&self) {
        self.send(UIEvent::ResetCursor);
    }

    pub fn restore_context(&self) {
        self.send(UIEvent::RestoreContext);
    }

    pub fn seek(&self, target: Timestamp, flags: gst::SeekFlags) {
        self.send(UIEvent::Seek { target, flags });
    }

    pub fn set_cursor_double_arrow(&self) {
        self.send(UIEvent::SetCursorDoubleArrow);
    }

    pub fn set_cursor_waiting(&self) {
        self.send(UIEvent::SetCursorWaiting);
    }

    pub fn show_all(&self) {
        self.send(UIEvent::ShowAll);
    }

    pub fn show_error<Msg>(&self, msg: Msg)
    where
        Msg: Into<Cow<'static, str>>,
    {
        self.send(UIEvent::ShowError(msg.into()));
    }

    pub fn show_info<Msg>(&self, msg: Msg)
    where
        Msg: Into<Cow<'static, str>>,
    {
        self.send(UIEvent::ShowInfo(msg.into()));
    }

    pub fn step_back(&self) {
        self.send(UIEvent::StepBack);
    }

    pub fn step_forward(&self) {
        self.send(UIEvent::StepForward);
    }

    pub fn stream_clicked(&self, type_: gst::StreamType) {
        self.send(UIEvent::StreamClicked(type_));
    }

    pub fn stream_export_toggled(&self, type_: gst::StreamType, tree_path: gtk::TreePath) {
        self.send(UIEvent::StreamExportToggled(type_, tree_path));
    }

    pub fn switch_to(&self, ctx: UIFocusContext) {
        self.send(UIEvent::SwitchTo(ctx));
    }

    // Call `restore_context` to retrieve initial state
    pub fn temporarily_switch_to(&self, ctx: UIFocusContext) {
        self.send(UIEvent::TemporarilySwitchTo(ctx));
    }

    pub fn tick(&self) {
        self.send(UIEvent::Tick);
    }

    pub fn toggle_chapter_list(&self, must_show: bool) {
        self.send(UIEvent::ToggleChapterList(must_show));
    }

    pub fn toggle_repeat(&self, must_repeat: bool) {
        self.send(UIEvent::ToggleRepeat(must_repeat));
    }

    pub fn update_audio_rendering_cndt(&self, dimensions: Option<(f64, f64)>) {
        self.send(UIEvent::UpdateAudioRenderingCndt { dimensions });
    }

    pub fn update_focus(&self) {
        self.send(UIEvent::UpdateFocus);
    }

    pub fn zoom_in(&self) {
        self.send(UIEvent::ZoomIn);
    }

    pub fn zoom_out(&self) {
        self.send(UIEvent::ZoomOut);
    }
}

pub(super) fn new_pair() -> (UIEventSender, async_mpsc::UnboundedReceiver<UIEvent>) {
    let (sender, receiver) = async_mpsc::unbounded();
    let sender = UIEventSender(RefCell::new(sender));

    (sender, receiver)
}
