use futures::{future::LocalBoxFuture, prelude::*};

use gtk::{gio, prelude::*};

use log::debug;

use crate::{info::ChapterEntry, main, playback, prelude::*};

pub struct Dispatcher;

impl UIDispatcher for Dispatcher {
    // FIXME use a dedicateds playback::Controller?
    type Controller = main::Controller;
    type Event = playback::Event;

    fn setup(main_ctrl: &mut main::Controller, app: &gtk::Application) {
        // Register Play/Pause action
        let play_pause = gio::SimpleAction::new("play_pause", None);
        app.add_action(&play_pause);
        play_pause.connect_activate(|_, _| playback::play_pause());
        main_ctrl.play_pause_btn.set_sensitive(true);
    }

    fn handle_event(
        main_ctrl: &mut main::Controller,
        event: impl Into<Self::Event>,
    ) -> LocalBoxFuture<'_, ()> {
        let event = event.into();
        async move {
            use playback::Event::*;

            debug!("handling {:?}", event);
            match event {
                Eos => main_ctrl.eos().await,
                NextChapter => {
                    let seek_ts = main_ctrl
                        .info
                        .chapter_manager
                        .pick_next()
                        .as_ref()
                        .map(ChapterEntry::start);

                    if let Some(seek_ts) = seek_ts {
                        let _ = main_ctrl.seek(seek_ts, gst::SeekFlags::ACCURATE).await;
                    }
                }
                PlayPause => main_ctrl.play_pause().await,
                PlayRange {
                    start,
                    end,
                    ts_to_restore,
                } => {
                    main_ctrl.play_range(start, end, ts_to_restore);
                }
                PreviousChapter => {
                    let seek_ts = main_ctrl
                        .current_ts()
                        .and_then(|cur_ts| main_ctrl.info.previous_chapter(cur_ts));

                    let _ = main_ctrl
                        .seek(seek_ts.unwrap_or_default(), gst::SeekFlags::ACCURATE)
                        .await;
                }
                ClearSeek => {
                    main_ctrl.seek_manager = playback::SeekManager::default();
                }
                SeekRequest { target, flags } => {
                    if main_ctrl.seek_manager.can_seek_now(target, flags) {
                        let _ = main_ctrl.seek(target, flags).await;
                    }
                }
                Seek { target, flags } => {
                    let _ = main_ctrl.seek(target, flags).await;
                }
            }
        }
        .boxed_local()
    }

    fn bind_accels_for(ctx: UIFocusContext, app: &gtk::Application) {
        use UIFocusContext::*;

        match ctx {
            PlaybackPage => {
                app.set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);
                app.set_accels_for_action("app.next_chapter", &["Down", "AudioNext"]);
                app.set_accels_for_action("app.previous_chapter", &["Up", "AudioPrev"]);
            }
            ExportPage | SplitPage | StreamsPage => {
                app.set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);
                app.set_accels_for_action("app.next_chapter", &["AudioNext"]);
                app.set_accels_for_action("app.previous_chapter", &["AudioPrev"]);
            }
            TextEntry => {
                app.set_accels_for_action("app.play_pause", &["AudioPlay"]);
                app.set_accels_for_action("app.next_chapter", &[]);
                app.set_accels_for_action("app.previous_chapter", &[]);
            }
            InfoBar => {
                app.set_accels_for_action("app.play_pause", &["AudioPlay"]);
                app.set_accels_for_action("app.next_chapter", &[]);
                app.set_accels_for_action("app.previous_chapter", &[]);
            }
        }
    }
}
