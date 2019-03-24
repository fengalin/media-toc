use crate::media::PlaybackPipeline;
use glib;
use gstreamer as gst;
use gtk;
use std::{borrow::Cow, rc::Rc};

#[derive(Clone)]
pub enum UIEvent {
    AskQuestion {
        question: Cow<'static, str>,
        response_cb: Rc<Fn(gtk::ResponseType)>,
    },
    HandBackPipeline(PlaybackPipeline),
    PlayRange {
        start: u64,
        end: u64,
        pos_to_restore: u64,
    },
    ResetCursor,
    Seek {
        position: u64,
        flags: gst::SeekFlags,
    },
    SetCursorWaiting,
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

    pub fn hand_back_pipeline(&self, pipeline: PlaybackPipeline) {
        self.0.send(UIEvent::HandBackPipeline(pipeline));
    }

    pub fn play_range(&self, start: u64, end: u64, pos_to_restore: u64) {
        self.0.send(UIEvent::PlayRange {
            start,
            end,
            pos_to_restore,
        });
    }

    pub fn reset_cursor(&self) {
        self.0.send(UIEvent::ResetCursor);
    }

    pub fn seek(&self, position: u64, flags: gst::SeekFlags) {
        self.0.send(UIEvent::Seek { position, flags });
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
