use gio;
use gio::prelude::*;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use crate::application::CONFIG;

use super::{MainController, UIDispatcher};

pub struct InfoDispatcher;
impl UIDispatcher for InfoDispatcher {
    fn setup(gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>) {
        let mut main_ctrl = main_ctrl_rc.borrow_mut();
        let info_ctrl = &mut main_ctrl.info_ctrl;

        // Register Toggle show chapters list action
        let toggle_show_list = gio::SimpleAction::new("toggle_show_list", None);
        gtk_app.add_action(&toggle_show_list);
        let show_chapters_btn = info_ctrl.show_chapters_btn.clone();
        toggle_show_list.connect_activate(move |_, _| {
            show_chapters_btn.set_active(!show_chapters_btn.get_active());
        });
        gtk_app.set_accels_for_action("app.toggle_show_list", &["l"]);

        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        info_ctrl
            .show_chapters_btn
            .connect_toggled(move |toggle_button| {
                let main_ctrl = main_ctrl_rc_cb.borrow();
                let info_ctrl = &main_ctrl.info_ctrl;
                if toggle_button.get_active() {
                    CONFIG.write().unwrap().ui.is_chapters_list_hidden = false;
                    info_ctrl.info_container.show();
                } else {
                    CONFIG.write().unwrap().ui.is_chapters_list_hidden = true;
                    info_ctrl.info_container.hide();
                }
            });

        // Draw thumnail image
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        info_ctrl
            .drawingarea
            .connect_draw(move |drawingarea, cairo_ctx| {
                let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
                main_ctrl.info_ctrl.draw_thumbnail(drawingarea, cairo_ctx);
                Inhibit(true)
            });

        // Scale seek
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        info_ctrl
            .timeline_scale
            .connect_change_value(move |_, _, value| {
                main_ctrl_rc_cb
                    .borrow_mut()
                    .seek(value as u64, gst::SeekFlags::KEY_UNIT);
                Inhibit(true)
            });

        // TreeView seek
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        info_ctrl
            .chapter_treeview
            .connect_row_activated(move |_, tree_path, _| {
                let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
                let info_ctrl = &mut main_ctrl.info_ctrl;
                if let Some(iter) = info_ctrl.chapter_manager.get_iter(tree_path) {
                    let position = info_ctrl.chapter_manager.get_chapter_at_iter(&iter).start();
                    // update position
                    info_ctrl.tick(position, false);
                    main_ctrl.seek(position, gst::SeekFlags::ACCURATE);
                }
            });

        // TreeView title modified
        if let Some(ref title_renderer) = info_ctrl.chapter_manager.title_renderer {
            let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
            title_renderer.connect_edited(move |_, _tree_path, new_title| {
                let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
                main_ctrl
                    .info_ctrl
                    .chapter_manager
                    .rename_selected_chapter(new_title);
                // reflect title modification in other parts of the UI (audio waveform)
                main_ctrl.refresh();
            });
        }

        // Register add chapter action
        let add_chapter = gio::SimpleAction::new("add_chapter", None);
        gtk_app.add_action(&add_chapter);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        add_chapter.connect_activate(move |_, _| {
            let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
            let position = main_ctrl.get_position();
            main_ctrl.info_ctrl.add_chapter(position);
        });
        gtk_app.set_accels_for_action("app.add_chapter", &["plus", "KP_Add"]);

        // Register remove chapter action
        let remove_chapter = gio::SimpleAction::new("remove_chapter", None);
        gtk_app.add_action(&remove_chapter);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        remove_chapter.connect_activate(move |_, _| {
            main_ctrl_rc_cb.borrow_mut().info_ctrl.remove_chapter();
        });
        gtk_app.set_accels_for_action("app.remove_chapter", &["minus", "KP_Subtract"]);

        // Register Toggle repeat current chapter action
        let toggle_repeat_chapter = gio::SimpleAction::new("toggle_repeat_chapter", None);
        gtk_app.add_action(&toggle_repeat_chapter);
        let repeat_btn = info_ctrl.repeat_btn.clone();
        toggle_repeat_chapter.connect_activate(move |_, _| {
            repeat_btn.set_active(!repeat_btn.get_active());
        });
        gtk_app.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);

        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        info_ctrl.repeat_btn.connect_clicked(move |button| {
            main_ctrl_rc_cb.borrow_mut().info_ctrl.repeat_chapter = button.get_active();
        });

        // Register next chapter action
        let next_chapter = gio::SimpleAction::new("next_chapter", None);
        gtk_app.add_action(&next_chapter);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        next_chapter.connect_activate(move |_, _| {
            let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
            let seek_pos = main_ctrl
                .info_ctrl
                .chapter_manager
                .next_iter()
                .map(|next_iter| {
                    main_ctrl
                        .info_ctrl
                        .chapter_manager
                        .get_chapter_at_iter(&next_iter)
                        .start()
                });

            if let Some(seek_pos) = seek_pos {
                main_ctrl.seek(seek_pos, gst::SeekFlags::ACCURATE);
            }
        });
        gtk_app.set_accels_for_action("app.next_chapter", &["Down", "AudioNext"]);

        // Register previous chapter action
        let previous_chapter = gio::SimpleAction::new("previous_chapter", None);
        gtk_app.add_action(&previous_chapter);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        previous_chapter.connect_activate(move |_, _| {
            let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
            let position = main_ctrl.get_position();
            let seek_pos = main_ctrl.info_ctrl.previous_pos(position);
            main_ctrl.seek(seek_pos, gst::SeekFlags::ACCURATE);
        });
        gtk_app.set_accels_for_action("app.previous_chapter", &["Up", "AudioPrev"]);

        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        info_ctrl.seek_fn = Some(Rc::new(move |position, seek_flags| {
            main_ctrl_rc_cb.borrow_mut().seek(position, seek_flags);
        }));

        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        info_ctrl.show_msg_fn = Some(Rc::new(move |msg_type, msg| {
            main_ctrl_rc_cb.borrow().show_message(msg_type, msg);
        }));
    }
}
