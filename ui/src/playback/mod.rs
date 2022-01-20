use futures::future::{abortable, AbortHandle};
use std::time::{Duration, Instant};

mod dispatcher;
pub use self::dispatcher::Dispatcher;

use renderers::Timestamp;

use crate::UIEventChannel;

#[derive(Debug)]
pub enum Event {
    Eos,
    NextChapter,
    PreviousChapter,
    PlayPause,
    PlayRange {
        start: Timestamp,
        end: Timestamp,
        ts_to_restore: Timestamp,
    },
    ClearSeek,
    Seek {
        target: Timestamp,
        flags: gst::SeekFlags,
    },
    SeekRequest {
        target: Timestamp,
        flags: gst::SeekFlags,
    },
}

pub fn eos() {
    UIEventChannel::send(Event::Eos);
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
    UIEventChannel::send(Event::SeekRequest {
        target: target.into(),
        flags,
    });
}

/// Manages seek.
///
/// This is used to prevent flooding the Playback pipeline
/// with successive seeks (e.g. using the timeline).
/// If more than MAX_SUCCESSIVE seeks are triggered:
///
/// - The seek state switches to DelayedSeek.
/// - The last seek is delayed for DEBOUNCE_DELAY.
/// - During that delay, if another seek is received, it
///   replaces previous last seek and it is also delayed.
/// - When the last delay has elapsed, the last seek is
///   triggered.
#[derive(Debug)]
pub enum SeekManager {
    NotSeeking,
    Seeking {
        count: usize,
        abort_hdl: AbortHandle,
    },
    Delayed {
        abort_hdl: AbortHandle,
    },
}

impl SeekManager {
    pub(crate) const MAX_SUCCESSIVE: usize = 2;
    pub(crate) const DEBOUNCE_DELAY: Duration = Duration::from_millis(200);

    /// Checks if seek can be performed now.
    ///
    /// # Returns
    ///
    /// - `true` if seek can be handled immediately.
    /// - `false` if seek shouldn't be handled immediately,
    ///   in which case, the seek is automatically delayed.
    pub(crate) fn can_seek_now(&mut self, target: Timestamp, flags: gst::SeekFlags) -> bool {
        use SeekManager::*;

        match self {
            NotSeeking => {
                *self = Seeking {
                    count: 1,
                    abort_hdl: Self::delay(|| UIEventChannel::send(Event::ClearSeek), None),
                };

                true
            }
            Seeking { count, abort_hdl } if *count < Self::MAX_SUCCESSIVE => {
                *self = Seeking {
                    count: *count + 1,
                    abort_hdl: Self::delay(
                        || UIEventChannel::send(Event::ClearSeek),
                        Some(abort_hdl),
                    ),
                };

                true
            }
            Seeking { abort_hdl, .. } | Delayed { abort_hdl } => {
                *self = Delayed {
                    abort_hdl: Self::delay(
                        move || {
                            UIEventChannel::send(Event::Seek { target, flags });
                            UIEventChannel::send(Event::ClearSeek);
                        },
                        Some(abort_hdl),
                    ),
                };

                false
            }
        }
    }

    #[inline]
    fn delay(f: impl FnOnce() + 'static, prev: Option<&AbortHandle>) -> AbortHandle {
        let start = Instant::now();
        if let Some(abort_hdl) = prev {
            abort_hdl.abort();
        }

        let (abortable, abort) = abortable(async move {
            if let Some(delay) = Self::DEBOUNCE_DELAY.checked_sub(start.elapsed()) {
                gtk::glib::timeout_future(delay).await;
            }
            f();
        });
        crate::spawn(async {
            let _ = abortable.await;
        });

        abort
    }
}

impl Default for SeekManager {
    fn default() -> Self {
        SeekManager::NotSeeking
    }
}
