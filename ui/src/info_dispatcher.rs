use gio::prelude::*;
use glib::clone;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use super::{InfoController, MainController, UIDispatcher, UIEventSender, UIFocusContext};

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
            @strong ui_event => move |toggle_button| {
                ui_event.toggle_chapter_list(!toggle_button.get_active());
            }
        ));

        // Draw thumnail image
        info_ctrl.drawingarea.connect_draw(clone!(
            @weak main_ctrl_rc => @default-panic, move |drawingarea, cairo_ctx| {
                if let Ok(mut main_ctrl) = main_ctrl_rc.try_borrow_mut() {
                    main_ctrl.info_ctrl.draw_thumbnail(drawingarea, cairo_ctx);
                }
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
            clone!(@strong ui_event => move |_, tree_path, _| {
                ui_event.chapter_clicked(tree_path.clone());
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
            .connect_clicked(clone!(@strong ui_event => move |button| {
                ui_event.toggle_repeat(button.get_active());
            }));

        // Register next chapter action
        app.add_action(&info_ctrl.next_chapter_action);
        info_ctrl
            .next_chapter_action
            .connect_activate(clone!(@strong ui_event => move |_, _| {
                ui_event.next_chapter();
            }));

        // Register previous chapter action
        app.add_action(&info_ctrl.previous_chapter_action);
        info_ctrl.previous_chapter_action.connect_activate(clone!(
            @strong ui_event => move |_, _| {
                ui_event.previous_chapter();
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
