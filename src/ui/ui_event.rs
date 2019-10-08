use futures::channel::mpsc as async_mpsc;
use futures::channel::oneshot;
use futures::prelude::*;

use gdk::{Cursor, CursorType, WindowExt};
use glib;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;
use std::{
    borrow::Cow,
    cell::{Ref, RefCell, RefMut},
    path::PathBuf,
    rc::Rc,
};

use media::Timestamp;

use super::{
    AudioDispatcher, ExportDispatcher, InfoBarController, InfoDispatcher, MainController,
    PerspectiveDispatcher, SplitDispatcher, StreamsDispatcher, UIController, UIDispatcher,
    VideoDispatcher,
};

const UI_EVENT_CHANNEL_CAPACITY: usize = 4;

#[derive(Clone, Copy)]
pub enum UIFocusContext {
    ExportPage,
    InfoBar,
    PlaybackPage,
    SplitPage,
    StreamsPage,
    TextEntry,
}

enum UIEvent {
    AskQuestion {
        question: Cow<'static, str>,
        response_sender: oneshot::Sender<gtk::ResponseType>,
    },
    CancelSelectMedia,
    OpenMedia(PathBuf),
    PlayRange {
        start: Timestamp,
        end: Timestamp,
        ts_to_restore: Timestamp,
    },
    Quit,
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

    pub async fn ask_question<Q>(&self, question: Q) -> gtk::ResponseType
    where
        Q: Into<Cow<'static, str>>,
    {
        let (response_sender, response_receiver) = oneshot::channel();
        self.send(UIEvent::AskQuestion {
            question: question.into(),
            response_sender,
        });

        response_receiver
            .await
            .expect("UI failed to receive response")
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

    pub fn quit(&self) {
        self.send(UIEvent::Quit);
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

pub struct UIEventHandler {
    receiver: async_mpsc::Receiver<UIEvent>,
    app: gtk::Application,
    window: gtk::ApplicationWindow,
    main_ctrl: Option<Rc<RefCell<MainController>>>,
    info_bar_ctrl: InfoBarController,
    saved_context: Option<UIFocusContext>,
    focus: UIFocusContext,
}

impl UIEventHandler {
    pub fn new_pair(app: &gtk::Application, builder: &gtk::Builder) -> (Self, UIEventSender) {
        let (sender, receiver) = async_mpsc::channel(UI_EVENT_CHANNEL_CAPACITY);
        let ui_event_sender = UIEventSender(RefCell::new(sender));

        let handler = UIEventHandler {
            receiver,
            app: app.clone(),
            window: builder.get_object("application-window").unwrap(),
            main_ctrl: None,
            info_bar_ctrl: InfoBarController::new(app, builder, &ui_event_sender),
            saved_context: None,
            focus: UIFocusContext::PlaybackPage,
        };

        (handler, ui_event_sender)
    }

    pub fn spawn(mut self, main_ctrl: &Rc<RefCell<MainController>>) {
        self.main_ctrl = Some(Rc::clone(&main_ctrl));

        spawn!(async move {
            while let Some(event) = self.receiver.next().await {
                if self.handle(event).is_err() {
                    break;
                }
            }
        });
    }

    #[inline]
    fn main_ctrl(&self) -> Ref<'_, MainController> {
        self.main_ctrl.as_ref().unwrap().borrow()
    }

    #[inline]
    fn main_ctrl_mut(&self) -> RefMut<'_, MainController> {
        self.main_ctrl.as_ref().unwrap().borrow_mut()
    }

    fn handle(&mut self, event: UIEvent) -> Result<(), ()> {
        match event {
            UIEvent::AskQuestion {
                question,
                response_sender,
            } => self.info_bar_ctrl.ask_question(&question, response_sender),
            UIEvent::CancelSelectMedia => self.main_ctrl_mut().cancel_select_media(),
            UIEvent::OpenMedia(path) => {
                self.info_bar_ctrl.hide();
                self.main_ctrl_mut().open_media(path);
            }
            UIEvent::PlayRange {
                start,
                end,
                ts_to_restore,
            } => {
                self.main_ctrl_mut().play_range(start, end, ts_to_restore);
            }
            UIEvent::Quit => {
                self.main_ctrl_mut().quit();
                return Err(());
            }
            UIEvent::ResetCursor => self.reset_cursor(),
            UIEvent::RestoreContext => self.restore_context(),
            UIEvent::ShowAll => self.show_all(),
            UIEvent::Seek { target, flags } => self.main_ctrl_mut().seek(target, flags),
            UIEvent::SetCursorDoubleArrow => self.set_cursor_double_arrow(),
            UIEvent::SetCursorWaiting => self.set_cursor_waiting(),
            UIEvent::ShowError(msg) => self.info_bar_ctrl.show_error(&msg),
            UIEvent::ShowInfo(msg) => self.info_bar_ctrl.show_info(&msg),
            UIEvent::SwitchTo(focus_ctx) => self.switch_to(focus_ctx),
            UIEvent::TemporarilySwitchTo(focus_ctx) => {
                self.save_context();
                self.bind_accels_for(focus_ctx);
            }
            UIEvent::UpdateFocus => self.update_focus(),
        }

        Ok(())
    }

    pub fn show_all(&self) {
        self.window.show();
        self.window.activate();
    }

    fn set_cursor_waiting(&self) {
        if let Some(gdk_window) = self.window.get_window() {
            gdk_window.set_cursor(Some(&Cursor::new_for_display(
                &gdk_window.get_display(),
                CursorType::Watch,
            )));
        }
    }

    fn set_cursor_double_arrow(&self) {
        if let Some(gdk_window) = self.window.get_window() {
            gdk_window.set_cursor(Some(&Cursor::new_for_display(
                &gdk_window.get_display(),
                CursorType::SbHDoubleArrow,
            )));
        }
    }

    fn reset_cursor(&self) {
        if let Some(gdk_window) = self.window.get_window() {
            gdk_window.set_cursor(None);
        }
    }

    fn bind_accels_for(&self, ctx: UIFocusContext) {
        match ctx {
            UIFocusContext::PlaybackPage => {
                self.app
                    .set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);
                self.app
                    .set_accels_for_action("app.next_chapter", &["Down", "AudioNext"]);
                self.app
                    .set_accels_for_action("app.previous_chapter", &["Up", "AudioPrev"]);
                self.app.set_accels_for_action("app.close_info_bar", &[]);
            }
            UIFocusContext::ExportPage
            | UIFocusContext::SplitPage
            | UIFocusContext::StreamsPage => {
                self.app
                    .set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);
                self.app
                    .set_accels_for_action("app.next_chapter", &["AudioNext"]);
                self.app
                    .set_accels_for_action("app.previous_chapter", &["AudioPrev"]);
                self.app.set_accels_for_action("app.close_info_bar", &[]);
            }
            UIFocusContext::TextEntry => {
                self.app
                    .set_accels_for_action("app.play_pause", &["AudioPlay"]);
                self.app.set_accels_for_action("app.next_chapter", &[]);
                self.app.set_accels_for_action("app.previous_chapter", &[]);
                self.app.set_accels_for_action("app.close_info_bar", &[]);
            }
            UIFocusContext::InfoBar => {
                self.app
                    .set_accels_for_action("app.play_pause", &["AudioPlay"]);
                self.app.set_accels_for_action("app.next_chapter", &[]);
                self.app.set_accels_for_action("app.previous_chapter", &[]);
                self.app
                    .set_accels_for_action("app.close_info_bar", &["Escape"]);
            }
        }

        PerspectiveDispatcher::bind_accels_for(ctx, &self.app);
        VideoDispatcher::bind_accels_for(ctx, &self.app);
        InfoDispatcher::bind_accels_for(ctx, &self.app);
        AudioDispatcher::bind_accels_for(ctx, &self.app);
        ExportDispatcher::bind_accels_for(ctx, &self.app);
        SplitDispatcher::bind_accels_for(ctx, &self.app);
        StreamsDispatcher::bind_accels_for(ctx, &self.app);
    }

    fn update_focus(&self) {
        let main_ctrl = self.main_ctrl();
        match self.focus {
            UIFocusContext::ExportPage => main_ctrl.export_ctrl.grab_focus(),
            UIFocusContext::PlaybackPage => main_ctrl.info_ctrl.grab_focus(),
            UIFocusContext::SplitPage => main_ctrl.split_ctrl.grab_focus(),
            UIFocusContext::StreamsPage => main_ctrl.streams_ctrl.grab_focus(),
            _ => (),
        }
    }

    fn switch_to(&mut self, ctx: UIFocusContext) {
        self.focus = ctx;
        self.bind_accels_for(ctx);
        self.update_focus();
    }

    fn save_context(&mut self) {
        self.saved_context = Some(self.focus);
    }

    fn restore_context(&mut self) {
        if let Some(focus_ctx) = self.saved_context.take() {
            self.switch_to(focus_ctx);
        }
    }
}
