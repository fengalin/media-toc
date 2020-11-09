mod dispatcher;
pub use self::dispatcher::Dispatcher;

use media::Timestamp;

use crate::UIEventChannel;

#[derive(Debug)]
pub enum Event {
    NextChapter,
    PreviousChapter,
    PlayPause,
    PlayRange {
        start: Timestamp,
        end: Timestamp,
        ts_to_restore: Timestamp,
    },
    Seek {
        target: Timestamp,
        flags: gst::SeekFlags,
    },
}

pub fn next_chapter() {
    UIEventChannel::send(Event::NextChapter);
}

pub fn previous_chapter() {
    UIEventChannel::send(Event::PreviousChapter);
}

pub fn play_pause() {
    UIEventChannel::send(Event::PlayPause);
}

pub fn play_range(start: Timestamp, end: Timestamp, ts_to_restore: Timestamp) {
    UIEventChannel::send(Event::PlayRange {
        start,
        end,
        ts_to_restore,
    });
}

pub fn seek(target: impl Into<Timestamp>, flags: gst::SeekFlags) {
    UIEventChannel::send(Event::Seek {
        target: target.into(),
        flags,
    });
}
