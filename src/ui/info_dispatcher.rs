use gio;
use gio::prelude::*;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use crate::application::CONFIG;

use crate::with_main_ctrl;

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
                    main_ctrl.seek(value as u64, gst::SeekFlags::KEY_UNIT);
                    Inhibit(true)
                }
            ));

        // TreeView seek
        info_ctrl
            .chapter_treeview
            .connect_row_activated(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, tree_path, _| {
                    let info_ctrl = &mut main_ctrl.info_ctrl;
                    if let Some(iter) = info_ctrl.chapter_manager.get_iter(tree_path) {
                        let position = info_ctrl.chapter_manager.get_chapter_at_iter(&iter).start();
                        // update position
                        info_ctrl.tick(position, false);
                        main_ctrl.seek(position, gst::SeekFlags::ACCURATE);
                    }
                }
            ));

        // TreeView title modified
        if let Some(ref title_renderer) = info_ctrl.chapter_manager.title_renderer {
            title_renderer.connect_edited(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _tree_path, new_title| {
                    main_ctrl
                        .info_ctrl
                        .chapter_manager
                        .rename_selected_chapter(new_title);
                    // reflect title modification in other parts of the UI (audio waveform)
                    main_ctrl.refresh();
                }
            ));
        }

        // Register add chapter action
        let add_chapter = gio::SimpleAction::new("add_chapter", None);
        gtk_app.add_action(&add_chapter);
        add_chapter.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| {
                let position = main_ctrl.get_position();
                main_ctrl.info_ctrl.add_chapter(position);
            }
        ));
        gtk_app.set_accels_for_action("app.add_chapter", &["plus", "KP_Add"]);

        // Register remove chapter action
        let remove_chapter = gio::SimpleAction::new("remove_chapter", None);
        gtk_app.add_action(&remove_chapter);
        remove_chapter.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| main_ctrl.info_ctrl.remove_chapter()
        ));
        gtk_app.set_accels_for_action("app.remove_chapter", &["minus", "KP_Subtract"]);

        // Register Toggle repeat current chapter action
        let toggle_repeat_chapter = gio::SimpleAction::new("toggle_repeat_chapter", None);
        gtk_app.add_action(&toggle_repeat_chapter);
        let repeat_btn = info_ctrl.repeat_btn.clone();
        toggle_repeat_chapter.connect_activate(move |_, _| {
            repeat_btn.set_active(!repeat_btn.get_active());
        });
        gtk_app.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);

        info_ctrl.repeat_btn.connect_clicked(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, button| {
                main_ctrl.info_ctrl.repeat_chapter = button.get_active();
            }
        ));

        // Register next chapter action
        let next_chapter = gio::SimpleAction::new("next_chapter", None);
        gtk_app.add_action(&next_chapter);
        next_chapter.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| {
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
            }
        ));

        // Register previous chapter action
        let previous_chapter = gio::SimpleAction::new("previous_chapter", None);
        gtk_app.add_action(&previous_chapter);
        previous_chapter.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| {
                let position = main_ctrl.get_position();
                let seek_pos = main_ctrl.info_ctrl.previous_pos(position);
                main_ctrl.seek(seek_pos, gst::SeekFlags::ACCURATE);
            }
        ));

        // activate Next chapter accels only when the display page is shown
        // so that other pages can use the Up & Down keys
        let gtk_app_cb = gtk_app.clone();
        main_ctrl.display_page.connect_map(move |_| {
            gtk_app_cb.set_accels_for_action("app.next_chapter", &["Down", "AudioNext"]);
            gtk_app_cb.set_accels_for_action("app.previous_chapter", &["Up", "AudioPrev"]);
        });

        let gtk_app_cb = gtk_app.clone();
        main_ctrl.display_page.connect_unmap(move |_| {
            gtk_app_cb.set_accels_for_action("app.next_chapter", &[]);
            gtk_app_cb.set_accels_for_action("app.previous_chapter", &[]);
        });
    }
}
