use glib;
use gstreamer as gst;
use gtk;
use std::{borrow::Cow, path::PathBuf, rc::Rc};

use crate::media::Timestamp;

#[derive(Clone)]
pub enum UIEvent {
    AskQuestion {
        question: Cow<'static, str>,
        response_cb: Rc<Fn(gtk::ResponseType)>,
    },
    CancelSelectMedia,
    OpenMedia(PathBuf),
    PlayRange {
        start: Timestamp,
        end: Timestamp,
        ts_to_restore: Timestamp,
    },
    ResetCursor,
    Seek {
        target: Timestamp,
        flags: gst::SeekFlags,
    },
    ShowAll,
    SetCursorWaiting,
    SetCursorDoubleArrow,
    ShowError(Cow<'static, str>),
    ShowInfo(Cow<'static, str>),
}

#[derive(Clone)]
pub struct UIEventSender(glib::Sender<UIEvent>);

#[allow(unused_must_use)]
impl UIEventSender {
    pub fn ask_question<Q>(&self, question: Q, response_cb: Rc<Fn(gtk::ResponseType)>)
    where
        Q: Into<Cow<'static, str>>,
    {
        self.0.send(UIEvent::AskQuestion {
            question: question.into(),
            response_cb,
        });
    }

    pub fn cancel_select_media(&self) {
        self.0.send(UIEvent::CancelSelectMedia);
    }

    pub fn open_media(&self, path: PathBuf) {
        // Trigger the message asynchronously otherwise the waiting cursor might not show up
        let mut path = Some(path);
        let sender = self.0.clone();
        gtk::idle_add(move || {
            if let Some(path) = path.take() {
                sender.send(UIEvent::OpenMedia(path));
            }
            glib::Continue(false)
        });
    }

    pub fn play_range(&self, start: Timestamp, end: Timestamp, ts_to_restore: Timestamp) {
        self.0.send(UIEvent::PlayRange {
            start,
            end,
            ts_to_restore,
        });
    }

    pub fn reset_cursor(&self) {
        self.0.send(UIEvent::ResetCursor);
    }

    pub fn show_all(&self) {
        self.0.send(UIEvent::ShowAll);
    }

    pub fn seek(&self, target: Timestamp, flags: gst::SeekFlags) {
        self.0.send(UIEvent::Seek { target, flags });
    }

    pub fn set_cursor_double_arrow(&self) {
        self.0.send(UIEvent::SetCursorDoubleArrow);
    }

    pub fn set_cursor_waiting(&self) {
        self.0.send(UIEvent::SetCursorWaiting);
    }

    pub fn show_error<Msg>(&self, msg: Msg)
    where
        Msg: Into<Cow<'static, str>>,
    {
        self.0.send(UIEvent::ShowError(msg.into()));
    }

    pub fn show_info<Msg>(&self, msg: Msg)
    where
        Msg: Into<Cow<'static, str>>,
    {
        self.0.send(UIEvent::ShowInfo(msg.into()));
    }
}

impl From<glib::Sender<UIEvent>> for UIEventSender {
    fn from(glib_ui_event: glib::Sender<UIEvent>) -> Self {
        UIEventSender(glib_ui_event)
    }
}
