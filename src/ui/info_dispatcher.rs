use gio;
use gio::prelude::*;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use super::{InfoController, MainController, UIDispatcher};
use crate::{application::CONFIG, with_main_ctrl};
use media::Timestamp;
use metadata::Duration;

const GO_TO_PREV_CHAPTER_THRESHOLD: Duration = Duration::from_secs(1);

pub struct InfoDispatcher;
impl UIDispatcher for InfoDispatcher {
    type Controller = InfoController;

    fn setup(
        info_ctrl: &mut InfoController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        app: &gtk::Application,
    ) {
        // Register Toggle show chapters list action
        let toggle_show_list = gio::SimpleAction::new("toggle_show_list", None);
        app.add_action(&toggle_show_list);
        let show_chapters_btn = info_ctrl.show_chapters_btn.clone();
        toggle_show_list.connect_activate(move |_, _| {
            show_chapters_btn.set_active(!show_chapters_btn.get_active());
        });
        app.set_accels_for_action("app.toggle_show_list", &["l"]);

        info_ctrl.show_chapters_btn.connect_toggled(with_main_ctrl!(
            main_ctrl_rc => move |&main_ctrl, toggle_button| {
                let info_ctrl = &main_ctrl.info_ctrl;
                if toggle_button.get_active() {
                    CONFIG.write().unwrap().ui.is_chapters_list_hidden = false;
                    info_ctrl.info_container.show();
                } else {
                    CONFIG.write().unwrap().ui.is_chapters_list_hidden = true;
                    info_ctrl.info_container.hide();
                }
            }
        ));

        // Draw thumnail image
        info_ctrl.drawingarea.connect_draw(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, drawingarea, cairo_ctx| {
                main_ctrl.info_ctrl.draw_thumbnail(drawingarea, cairo_ctx);
                Inhibit(true)
            }
        ));

        // Scale seek
        info_ctrl
            .timeline_scale
            .connect_change_value(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _, value| {
                    main_ctrl.seek((value as u64).into(), gst::SeekFlags::KEY_UNIT);
                    Inhibit(true)
                }
            ));

        // TreeView seek
        info_ctrl
            .chapter_treeview
            .connect_row_activated(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, tree_path, _| {
                    let info_ctrl = &mut main_ctrl.info_ctrl;
                    if let Some(chapter) = &info_ctrl.chapter_manager.chapter_from_path(tree_path) {
                        let seek_ts = chapter.start();
                        main_ctrl.seek(seek_ts, gst::SeekFlags::ACCURATE);
                    }
                }
            ));

        if let Some(ref title_renderer) = info_ctrl.chapter_manager.title_renderer {
            // FIXME: when editing is canceled, we should restore the title that was used when editing starts...
            let app_cb = app.clone();
            title_renderer.connect_editing_started(move |_, _, _| {
                // FIXME: use a message to disable accels for printable chars & arrow keys (text entry significant keys)
                app_cb.set_accels_for_action("app.play_pause", &["AudioPlay"]);
                // Shouldn't need to handle "Escape" once it is properly handled
                // once info_bar is opened/closed
                app_cb.set_accels_for_action("app.close_info_bar", &[]);
                app_cb.set_accels_for_action("app.toggle_show_list", &[]);
                app_cb.set_accels_for_action("app.toggle_repeat_chapter", &[]);
                app_cb.set_accels_for_action("app.add_chapter", &["KP_Add"]);
                app_cb.set_accels_for_action("app.remove_chapter", &["KP_Subtract"]);
                app_cb.set_accels_for_action("app.zoom_in", &[]);
                app_cb.set_accels_for_action("app.step_forward", &[]);
                app_cb.set_accels_for_action("app.step_back", &[]);
            });

            let app_cb = app.clone();
            title_renderer.connect_editing_canceled(move |_| {
                // FIXME: use a message to enable accels for printable chars & arrow keys (text entry significant keys)
                app_cb.set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);
                // Shouldn't need to handle "Escape" once it is properly handled
                // once info_bar is opened/closed
                app_cb.set_accels_for_action("app.close_info_bar", &["Escape"]);
                app_cb.set_accels_for_action("app.toggle_show_list", &["l"]);
                app_cb.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);
                app_cb.set_accels_for_action("app.add_chapter", &["plus", "KP_Add"]);
                app_cb.set_accels_for_action("app.remove_chapter", &["minus", "KP_Subtract"]);
                app_cb.set_accels_for_action("app.zoom_in", &["z"]);
                app_cb.set_accels_for_action("app.step_forward", &["Right"]);
                app_cb.set_accels_for_action("app.step_back", &["Left"]);
            });

            let app_cb = app.clone();
            title_renderer.connect_edited(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _tree_path, new_title| {
                    // FIXME: use a message to enable accels for printable chars & arrow keys (text entry significant keys)
                    app_cb.set_accels_for_action("app.play_pause", &["space", "AudioPlay"]);
                    // Shouldn't need to handle "Escape" once it is properly handled
                    // once info_bar is opened/closed
                    app_cb.set_accels_for_action("app.close_info_bar", &["Escape"]);
                    app_cb.set_accels_for_action("app.toggle_show_list", &["l"]);
                    app_cb.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);
                    app_cb.set_accels_for_action("app.add_chapter", &["plus", "KP_Add"]);
                    app_cb.set_accels_for_action("app.remove_chapter", &["minus", "KP_Subtract"]);
                    app_cb.set_accels_for_action("app.zoom_in", &["z"]);
                    app_cb.set_accels_for_action("app.step_forward", &["Right"]);
                    app_cb.set_accels_for_action("app.step_back", &["Left"]);
                    main_ctrl
                        .info_ctrl
                        .chapter_manager
                        .rename_selected(new_title);
                    // reflect title modification in other parts of the UI (audio waveform)
                    main_ctrl.refresh();
                }
            ));
        }

        // Register add chapter action
        let add_chapter = gio::SimpleAction::new("add_chapter", None);
        app.add_action(&add_chapter);
        add_chapter.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| {
                let ts = main_ctrl.get_current_ts();
                main_ctrl.info_ctrl.add_chapter(ts);
            }
        ));
        app.set_accels_for_action("app.add_chapter", &["plus", "KP_Add"]);

        // Register remove chapter action
        let remove_chapter = gio::SimpleAction::new("remove_chapter", None);
        app.add_action(&remove_chapter);
        remove_chapter.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| main_ctrl.info_ctrl.remove_chapter()
        ));
        app.set_accels_for_action("app.remove_chapter", &["minus", "KP_Subtract"]);

        // Register Toggle repeat current chapter action
        let toggle_repeat_chapter = gio::SimpleAction::new("toggle_repeat_chapter", None);
        app.add_action(&toggle_repeat_chapter);
        let repeat_btn = info_ctrl.repeat_btn.clone();
        toggle_repeat_chapter.connect_activate(move |_, _| {
            repeat_btn.set_active(!repeat_btn.get_active());
        });
        app.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);

        info_ctrl.repeat_btn.connect_clicked(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, button| {
                main_ctrl.info_ctrl.repeat_chapter = button.get_active();
            }
        ));

        // Register next chapter action
        let next_chapter = gio::SimpleAction::new("next_chapter", None);
        app.add_action(&next_chapter);
        next_chapter.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| {
                let seek_pos = main_ctrl
                    .info_ctrl
                    .chapter_manager
                    .pick_next()
                    .map(|next_chapter| next_chapter.start());

                if let Some(seek_pos) = seek_pos {
                    main_ctrl.seek(seek_pos, gst::SeekFlags::ACCURATE);
                }
            }
        ));

        // Register previous chapter action
        let previous_chapter = gio::SimpleAction::new("previous_chapter", None);
        app.add_action(&previous_chapter);
        previous_chapter.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| {
                let cur_ts = main_ctrl.get_current_ts();
                let cur_start = main_ctrl
                    .info_ctrl
                    .chapter_manager
                    .selected()
                    .map(|sel_chapter| sel_chapter.start());
                let prev_start = main_ctrl
                    .info_ctrl
                    .chapter_manager
                    .pick_previous()
                    .map(|prev_chapter| prev_chapter.start());

                let seek_ts = match (cur_start, prev_start) {
                    (Some(cur_start), prev_start_opt) => {
                        if cur_ts > cur_start + GO_TO_PREV_CHAPTER_THRESHOLD {
                            Some(cur_start)
                        } else {
                            prev_start_opt
                        }
                    }
                    (None, prev_start_opt) => prev_start_opt,
                };

                main_ctrl.seek(seek_ts.unwrap_or_else(Timestamp::default), gst::SeekFlags::ACCURATE);
            }
        ));
    }
}
