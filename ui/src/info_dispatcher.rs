use gio::prelude::*;
use glib::clone;
use gstreamer as gst;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use application::CONFIG;
use media::Timestamp;
use metadata::Duration;

use super::{InfoController, MainController, UIDispatcher, UIEventSender, UIFocusContext};

const GO_TO_PREV_CHAPTER_THRESHOLD: Duration = Duration::from_secs(1);

pub struct InfoDispatcher;
impl UIDispatcher for InfoDispatcher {
    type Controller = InfoController;

    fn setup(
        info_ctrl: &mut InfoController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        app: &gtk::Application,
        ui_event: &UIEventSender,
    ) {
        // Register Toggle show chapters list action
        let toggle_show_list = gio::SimpleAction::new("toggle_show_list", None);
        app.add_action(&toggle_show_list);
        let show_chapters_btn = info_ctrl.show_chapters_btn.clone();
        toggle_show_list.connect_activate(move |_, _| {
            show_chapters_btn.set_active(!show_chapters_btn.get_active());
        });

        info_ctrl.show_chapters_btn.connect_toggled(clone!(
            @weak main_ctrl_rc => move |toggle_button| {
                let main_ctrl = main_ctrl_rc.borrow();
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
        info_ctrl.drawingarea.connect_draw(clone!(
            @weak main_ctrl_rc => @default-panic, move |drawingarea, cairo_ctx| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.info_ctrl.draw_thumbnail(drawingarea, cairo_ctx);
                Inhibit(true)
            }
        ));

        // Scale seek
        info_ctrl.timeline_scale.connect_change_value(
            clone!(@weak main_ctrl_rc => @default-panic, move |_, _, value| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.seek((value as u64).into(), gst::SeekFlags::KEY_UNIT);
                Inhibit(true)
            }),
        );

        // TreeView seek
        info_ctrl.chapter_treeview.connect_row_activated(
            clone!(@weak main_ctrl_rc => move |_, tree_path, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                let info_ctrl = &mut main_ctrl.info_ctrl;
                if let Some(chapter) = &info_ctrl.chapter_manager.chapter_from_path(tree_path) {
                    let seek_ts = chapter.start();
                    main_ctrl.seek(seek_ts, gst::SeekFlags::ACCURATE);
                }
            }),
        );

        if let Some(ref title_renderer) = info_ctrl.chapter_manager.title_renderer {
            let ui_event_cb = ui_event.clone();
            title_renderer.connect_editing_started(move |_, _, _| {
                ui_event_cb.temporarily_switch_to(UIFocusContext::TextEntry);
            });

            let ui_event_cb = ui_event.clone();
            title_renderer.connect_editing_canceled(move |_| {
                ui_event_cb.restore_context();
            });

            title_renderer.connect_edited(clone!(
                @weak main_ctrl_rc => move |_, _tree_path, new_title| {
                    let mut main_ctrl = main_ctrl_rc.borrow_mut();
                    main_ctrl
                        .info_ctrl
                        .chapter_manager
                        .rename_selected(new_title);
                    // reflect title modification in other parts of the UI (audio waveform)
                    main_ctrl.redraw();
                    main_ctrl.ui_event().restore_context();
                }
            ));
        }

        // Register add chapter action
        app.add_action(&info_ctrl.add_chapter_action);
        info_ctrl
            .add_chapter_action
            .connect_activate(clone!(@weak main_ctrl_rc => move |_, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                if let Some(ts) = main_ctrl.current_ts() {
                    main_ctrl.info_ctrl.add_chapter(ts);
                    main_ctrl.ui_event().update_focus();
                }
            }));

        // Register remove chapter action
        app.add_action(&info_ctrl.del_chapter_action);
        info_ctrl
            .del_chapter_action
            .connect_activate(clone!(@weak main_ctrl_rc => move |_, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.info_ctrl.remove_chapter();
                main_ctrl.ui_event().update_focus();
            }));

        // Register Toggle repeat current chapter action
        let toggle_repeat_chapter = gio::SimpleAction::new("toggle_repeat_chapter", None);
        app.add_action(&toggle_repeat_chapter);
        let repeat_btn = info_ctrl.repeat_btn.clone();
        toggle_repeat_chapter.connect_activate(move |_, _| {
            repeat_btn.set_active(!repeat_btn.get_active());
        });

        info_ctrl
            .repeat_btn
            .connect_clicked(clone!(@weak main_ctrl_rc => move |button| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.info_ctrl.repeat_chapter = button.get_active();
            }));

        // Register next chapter action
        app.add_action(&info_ctrl.next_chapter_action);
        info_ctrl
            .next_chapter_action
            .connect_activate(clone!(@weak main_ctrl_rc => move |_, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                let seek_pos = main_ctrl
                    .info_ctrl
                    .chapter_manager
                    .pick_next()
                    .map(|next_chapter| next_chapter.start());

                if let Some(seek_pos) = seek_pos {
                    main_ctrl.seek(seek_pos, gst::SeekFlags::ACCURATE);
                }
            }));

        // Register previous chapter action
        app.add_action(&info_ctrl.previous_chapter_action);
        info_ctrl.previous_chapter_action.connect_activate(clone!(
            @weak main_ctrl_rc => move |_, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                if let Some(cur_ts) = main_ctrl.current_ts() {
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
            }
        ));
    }

    fn bind_accels_for(ctx: UIFocusContext, app: &gtk::Application) {
        match ctx {
            UIFocusContext::PlaybackPage => {
                app.set_accels_for_action("app.toggle_show_list", &["l"]);
                app.set_accels_for_action("app.add_chapter", &["plus", "KP_Add"]);
                app.set_accels_for_action("app.del_chapter", &["minus", "KP_Subtract"]);
                app.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);
            }
            UIFocusContext::ExportPage
            | UIFocusContext::SplitPage
            | UIFocusContext::StreamsPage => {
                app.set_accels_for_action("app.toggle_show_list", &["l"]);
                app.set_accels_for_action("app.add_chapter", &[]);
                app.set_accels_for_action("app.del_chapter", &[]);
                app.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);
            }
            UIFocusContext::TextEntry | UIFocusContext::InfoBar => {
                app.set_accels_for_action("app.toggle_show_list", &[]);
                app.set_accels_for_action("app.add_chapter", &[]);
                app.set_accels_for_action("app.del_chapter", &[]);
                app.set_accels_for_action("app.toggle_repeat_chapter", &[]);
            }
        }
    }
}
