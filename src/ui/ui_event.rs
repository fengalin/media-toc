use futures::channel::mpsc as async_mpsc;

use glib;
use gstreamer as gst;
use gtk;
use std::{borrow::Cow, cell::RefCell, path::PathBuf, rc::Rc};

use super::UIFocusContext;
use crate::spawn;
use media::Timestamp;

#[derive(Clone)]
pub enum UIEvent {
    AskQuestion {
        question: Cow<'static, str>,
        response_cb: Rc<dyn Fn(gtk::ResponseType)>,
    },
    CancelSelectMedia,
    OpenMedia(PathBuf),
    PlayRange {
        start: Timestamp,
        end: Timestamp,
        ts_to_restore: Timestamp,
    },
    ResetCursor,
    RestoreContext,
    Seek {
        target: Timestamp,
        flags: gst::SeekFlags,
    },
    ShowAll,
    SetCursorWaiting,
    SetCursorDoubleArrow,
    ShowError(Cow<'static, str>),
    ShowInfo(Cow<'static, str>),
    SwitchTo(UIFocusContext),
    TemporarilySwitchTo(UIFocusContext),
    UpdateFocus,
}

#[derive(Clone)]
pub struct UIEventSender(RefCell<async_mpsc::Sender<UIEvent>>);

#[allow(unused_must_use)]
impl UIEventSender {
    fn send(&self, event: UIEvent) {
        self.0.borrow_mut().try_send(event).unwrap();
    }

    pub fn ask_question<Q>(&self, question: Q, response_cb: Rc<dyn Fn(gtk::ResponseType)>)
    where
        Q: Into<Cow<'static, str>>,
    {
        self.send(UIEvent::AskQuestion {
            question: question.into(),
            response_cb,
        });
    }

    pub fn cancel_select_media(&self) {
        self.send(UIEvent::CancelSelectMedia);
    }

    pub fn open_media(&self, path: PathBuf) {
        // Trigger the message asynchronously otherwise the waiting cursor might not show up
        let mut path = Some(path);
        let mut sender = self.0.borrow_mut().clone();
        spawn!(async move {
            if let Some(path) = path.take() {
                sender.try_send(UIEvent::OpenMedia(path)).unwrap();
            }
        });
    }

    pub fn play_range(&self, start: Timestamp, end: Timestamp, ts_to_restore: Timestamp) {
        self.send(UIEvent::PlayRange {
            start,
            end,
            ts_to_restore,
        });
    }

    pub fn reset_cursor(&self) {
        self.send(UIEvent::ResetCursor);
    }

    pub fn restore_context(&self) {
        self.send(UIEvent::RestoreContext);
    }

    pub fn show_all(&self) {
        self.send(UIEvent::ShowAll);
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

    pub fn switch_to(&self, ctx: UIFocusContext) {
        self.send(UIEvent::SwitchTo(ctx));
    }

    // Call `restore_context` to retrieve initial state
    pub fn temporarily_switch_to(&self, ctx: UIFocusContext) {
        self.send(UIEvent::TemporarilySwitchTo(ctx));
    }

    pub fn update_focus(&self) {
        self.send(UIEvent::UpdateFocus);
    }
}

impl From<async_mpsc::Sender<UIEvent>> for UIEventSender {
    fn from(async_ui_event_sender: async_mpsc::Sender<UIEvent>) -> Self {
        UIEventSender(RefCell::new(async_ui_event_sender))
    }
}
