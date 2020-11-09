use futures::{
    future::{self, LocalBoxFuture},
    prelude::*,
};

use gio::prelude::*;
use gtk::prelude::*;

use log::debug;

use crate::{
    info::{self, ChapterEntry},
    main, playback,
    prelude::*,
};

pub struct Dispatcher;
impl UIDispatcher for Dispatcher {
    type Controller = info::Controller;
    type Event = info::Event;

    fn setup(info: &mut info::Controller, app: &gtk::Application) {
        // Register Toggle show chapters list action
        let toggle_show_list = gio::SimpleAction::new("toggle_show_list", None);
        app.add_action(&toggle_show_list);
        let show_chapters_btn = info.show_chapters_btn.clone();
        toggle_show_list.connect_activate(move |_, _| {
            show_chapters_btn.set_active(!show_chapters_btn.get_active());
        });

        info.show_chapters_btn.connect_toggled(|toggle_button| {
            info::toggle_chapter_list(!toggle_button.get_active());
        });

        // Scale seek
        info.timeline_scale.connect_change_value(|_, _, value| {
            playback::seek(value as u64, gst::SeekFlags::KEY_UNIT);
            Inhibit(false)
        });

        // TreeView seek
        info.chapter_treeview
            .connect_row_activated(|_, tree_path, _| {
                info::chapter_clicked(tree_path.clone());
            });

        if let Some(ref title_renderer) = info.chapter_manager.title_renderer {
            title_renderer.connect_editing_started(|_, _, _| {
                main::temporarily_switch_to(UIFocusContext::TextEntry);
            });

            title_renderer.connect_editing_canceled(|_| {
                main::restore_context();
            });

            title_renderer.connect_edited(|_, _tree_path, new_title| {
                info::rename_chapter(new_title);
                main::restore_context();
            });
        }

        // Register add chapter action
        app.add_action(&info.add_chapter_action);
        info.add_chapter_action.connect_activate(|_, _| {
            info::add_chapter();
            main::update_focus();
        });

        // Register remove chapter action
        app.add_action(&info.del_chapter_action);
        info.del_chapter_action.connect_activate(|_, _| {
            info::remove_chapter();
            main::update_focus();
        });

        // Register Toggle repeat current chapter action
        let toggle_repeat_chapter = gio::SimpleAction::new("toggle_repeat_chapter", None);
        app.add_action(&toggle_repeat_chapter);
        let repeat_btn = info.repeat_btn.clone();
        toggle_repeat_chapter.connect_activate(move |_, _| {
            repeat_btn.set_active(!repeat_btn.get_active());
        });

        info.repeat_btn
            .connect_clicked(|button| info::toggle_repeat(button.get_active()));

        // Register next chapter action
        app.add_action(&info.next_chapter_action);
        info.next_chapter_action
            .connect_activate(|_, _| playback::next_chapter());

        // Register previous chapter action
        app.add_action(&info.previous_chapter_action);
        info.previous_chapter_action
            .connect_activate(|_, _| playback::previous_chapter());
    }

    fn handle_event(
        main_ctrl: &mut main::Controller,
        event: impl Into<Self::Event>,
    ) -> LocalBoxFuture<'_, ()> {
        use info::Event::*;

        let event = event.into();
        debug!("handling {:?}", event);
        match event {
            AddChapter => {
                if let Some(ts) = main_ctrl.current_ts() {
                    main_ctrl.info.add_chapter(ts);
                }
            }
            ChapterClicked(chapter_path) => {
                let seek_ts = main_ctrl
                    .info
                    .chapter_manager
                    .chapter_from_path(&chapter_path)
                    .as_ref()
                    .map(ChapterEntry::start);

                if let Some(seek_ts) = seek_ts {
                    let _ = main_ctrl.seek(seek_ts, gst::SeekFlags::ACCURATE);
                }
            }
            Refresh(ts) => match main_ctrl.state {
                main::State::Seeking(_) => (),
                _ => main_ctrl.info.tick(ts, main_ctrl.state),
            },
            RemoveChapter => main_ctrl.info.remove_chapter(),
            RenameChapter(new_title) => {
                main_ctrl.info.chapter_manager.rename_selected(&new_title);
                // reflect title modification in other parts of the UI (audio waveform)
                main_ctrl.redraw();
            }
            ToggleChapterList(must_show) => main_ctrl.info.toggle_chapter_list(must_show),
            ToggleRepeat(must_repeat) => main_ctrl.info.repeat_chapter = must_repeat,
        }

        future::ready(()).boxed_local()
    }

    fn bind_accels_for(ctx: UIFocusContext, app: &gtk::Application) {
        use UIFocusContext::*;

        match ctx {
            PlaybackPage => {
                app.set_accels_for_action("app.toggle_show_list", &["l"]);
                app.set_accels_for_action("app.add_chapter", &["plus", "KP_Add"]);
                app.set_accels_for_action("app.del_chapter", &["minus", "KP_Subtract"]);
                app.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);
            }
            ExportPage | SplitPage | StreamsPage => {
                app.set_accels_for_action("app.toggle_show_list", &["l"]);
                app.set_accels_for_action("app.add_chapter", &[]);
                app.set_accels_for_action("app.del_chapter", &[]);
                app.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);
            }
            TextEntry | InfoBar => {
                app.set_accels_for_action("app.toggle_show_list", &[]);
                app.set_accels_for_action("app.add_chapter", &[]);
                app.set_accels_for_action("app.del_chapter", &[]);
                app.set_accels_for_action("app.toggle_repeat_chapter", &[]);
            }
        }
    }
}
