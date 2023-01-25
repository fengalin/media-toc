use futures::channel::mpsc as async_mpsc;

use std::cell::{Cell, RefCell};

use crate::{audio, export, info, info_bar, main_panel, playback, split, streams};

thread_local! {
    pub static UI_EVENT_CHANNEL: UIEventChannel = UIEventChannel::new();
}

pub struct UIEventChannel {
    sender: RefCell<async_mpsc::UnboundedSender<UIEvent>>,
    receiver: Cell<Option<async_mpsc::UnboundedReceiver<UIEvent>>>,
}

impl UIEventChannel {
    fn new() -> Self {
        let (sender, receiver) = async_mpsc::unbounded();
        UIEventChannel {
            sender: RefCell::new(sender),
            receiver: Cell::new(Some(receiver)),
        }
    }

    #[track_caller]
    pub(crate) fn take_receiver() -> async_mpsc::UnboundedReceiver<UIEvent> {
        UI_EVENT_CHANNEL.with(|this| this.receiver.take().unwrap())
    }

    pub fn send(event: impl Into<UIEvent>) {
        UI_EVENT_CHANNEL.with(|this| {
            let _ = this.sender.borrow_mut().unbounded_send(event.into());
        });
    }
}

#[derive(Debug)]
pub enum UIEvent {
    Audio(audio::Event),
    Export(export::Event),
    Info(info::Event),
    InfoBar(info_bar::Event),
    Main(main_panel::Event),
    Playback(playback::Event),
    Split(split::Event),
    Streams(streams::Event),
}

impl From<audio::Event> for UIEvent {
    fn from(event: audio::Event) -> Self {
        UIEvent::Audio(event)
    }
}

impl From<export::Event> for UIEvent {
    fn from(event: export::Event) -> Self {
        UIEvent::Export(event)
    }
}

impl From<info::Event> for UIEvent {
    fn from(event: info::Event) -> Self {
        UIEvent::Info(event)
    }
}

impl From<info_bar::Event> for UIEvent {
    fn from(event: info_bar::Event) -> Self {
        UIEvent::InfoBar(event)
    }
}

impl From<main_panel::Event> for UIEvent {
    fn from(event: main_panel::Event) -> Self {
        UIEvent::Main(event)
    }
}

impl From<playback::Event> for UIEvent {
    fn from(event: playback::Event) -> Self {
        UIEvent::Playback(event)
    }
}

impl From<split::Event> for UIEvent {
    fn from(event: split::Event) -> Self {
        UIEvent::Split(event)
    }
}

impl From<streams::Event> for UIEvent {
    fn from(event: streams::Event) -> Self {
        UIEvent::Streams(event)
    }
}
