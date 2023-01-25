mod controller;
pub use self::controller::{Controller, State};

mod dispatcher;
pub use self::dispatcher::Dispatcher;

use std::path::PathBuf;

use crate::{UIEventChannel, UIFocusContext};

#[derive(Debug)]
pub enum Event {
    About,
    CancelSelectMedia,
    OpenMedia(PathBuf),
    Quit,
    ResetCursor,
    RestoreContext,
    SelectMedia,
    SetCursorDoubleArrow,
    SetCursorWaiting,
    ShowAll,
    SwitchTo(UIFocusContext),
    TemporarilySwitchTo(UIFocusContext),
    UpdateFocus,
}

fn about() {
    UIEventChannel::send(Event::About);
}

fn cancel_select_media() {
    UIEventChannel::send(Event::CancelSelectMedia);
}

fn open_media(path: impl Into<PathBuf>) {
    UIEventChannel::send(Event::OpenMedia(path.into()));
}

pub fn quit() {
    UIEventChannel::send(Event::Quit);
}

pub fn reset_cursor() {
    UIEventChannel::send(Event::ResetCursor);
}

pub fn restore_context() {
    UIEventChannel::send(Event::RestoreContext);
}

fn select_media() {
    UIEventChannel::send(Event::SelectMedia);
}

pub fn set_cursor_double_arrow() {
    UIEventChannel::send(Event::SetCursorDoubleArrow);
}

pub fn set_cursor_waiting() {
    UIEventChannel::send(Event::SetCursorWaiting);
}

fn show_all() {
    UIEventChannel::send(Event::ShowAll);
}

pub fn switch_to(ctx: UIFocusContext) {
    UIEventChannel::send(Event::SwitchTo(ctx));
}

pub fn temporarily_switch_to(ctx: UIFocusContext) {
    UIEventChannel::send(Event::TemporarilySwitchTo(ctx));
}

pub fn update_focus() {
    UIEventChannel::send(Event::UpdateFocus);
}
