use cairo;
use gettextrs::gettext;
use gio;
use gio::prelude::*;
use glib;
use gtk;
use gtk::prelude::*;
use lazy_static::lazy_static;
use log::info;

use std::cell::RefCell;
use std::fs::File;
use std::rc::{Rc, Weak};

use crate::application::CONFIG;
use crate::media::PlaybackContext;
use crate::metadata;
use crate::metadata::{MediaInfo, Timestamp};

use super::{
    ChapterTreeManager, ChaptersBoundaries, ControllerState, ImageSurface, MainController,
};

const GO_TO_PREV_CHAPTER_THRESHOLD: u64 = 1_000_000_000; // 1 s

lazy_static! {
    static ref EMPTY_REPLACEMENT: String = "-".to_owned();
}

pub struct InfoController {
    info_container: gtk::Grid,
    show_chapters_btn: gtk::ToggleButton,

    drawingarea: gtk::DrawingArea,

    title_lbl: gtk::Label,
    artist_lbl: gtk::Label,
    container_lbl: gtk::Label,
    audio_codec_lbl: gtk::Label,
    video_codec_lbl: gtk::Label,
    duration_lbl: gtk::Label,

    timeline_scale: gtk::Scale,
    repeat_btn: gtk::ToggleToolButton,

    chapter_treeview: gtk::TreeView,
    add_chapter_btn: gtk::ToolButton,
    del_chapter_btn: gtk::ToolButton,

    thumbnail: Option<cairo::ImageSurface>,

    chapter_manager: ChapterTreeManager,

    duration: u64,
    repeat_chapter: bool,

    main_ctrl: Option<Weak<RefCell<MainController>>>,
}

impl InfoController {
    pub fn new(
        builder: &gtk::Builder,
        boundaries: Rc<RefCell<ChaptersBoundaries>>,
    ) -> Rc<RefCell<Self>> {
        let mut chapter_manager = ChapterTreeManager::new(
            builder.get_object("chapters-tree-store").unwrap(),
            boundaries,
        );
        let chapter_treeview: gtk::TreeView = builder.get_object("chapter-treeview").unwrap();
        chapter_manager.init_treeview(&chapter_treeview);

        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let this_rc = Rc::new(RefCell::new(InfoController {
            info_container: builder.get_object("info-chapter_list-grid").unwrap(),
            show_chapters_btn: builder.get_object("show_chapters-toggle").unwrap(),

            drawingarea: builder.get_object("thumbnail-drawingarea").unwrap(),

            title_lbl: builder.get_object("title-lbl").unwrap(),
            artist_lbl: builder.get_object("artist-lbl").unwrap(),
            container_lbl: builder.get_object("container-lbl").unwrap(),
            audio_codec_lbl: builder.get_object("audio_codec-lbl").unwrap(),
            video_codec_lbl: builder.get_object("video_codec-lbl").unwrap(),
            duration_lbl: builder.get_object("duration-lbl").unwrap(),

            timeline_scale: builder.get_object("timeline-scale").unwrap(),
            repeat_btn: builder.get_object("repeat-toolbutton").unwrap(),

            chapter_treeview,
            add_chapter_btn: builder.get_object("add_chapter-toolbutton").unwrap(),
            del_chapter_btn: builder.get_object("remove_chapter-toolbutton").unwrap(),

            thumbnail: None,

            chapter_manager,

            duration: 0,
            repeat_chapter: false,

            main_ctrl: None,
        }));

        {
            let mut this = this_rc.borrow_mut();
            this.cleanup();
        }

        this_rc
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        gtk_app: &gtk::Application,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();

        this.main_ctrl = Some(Rc::downgrade(main_ctrl));

        // Show chapters toggle
        if CONFIG.read().unwrap().ui.is_chapters_list_hidden {
            this.show_chapters_btn.set_active(false);
            this.info_container.hide();
        }

        // Register Toggle show chapters list action
        let toggle_show_list = gio::SimpleAction::new("toggle_show_list", None);
        gtk_app.add_action(&toggle_show_list);
        let show_chapters_btn = this.show_chapters_btn.clone();
        toggle_show_list.connect_activate(move |_, _| {
            show_chapters_btn.set_active(!show_chapters_btn.get_active());
        });
        gtk_app.set_accels_for_action("app.toggle_show_list", &["l"]);

        let this_clone = Rc::clone(this_rc);
        this.show_chapters_btn
            .connect_toggled(move |toggle_button| {
                if toggle_button.get_active() {
                    CONFIG.write().unwrap().ui.is_chapters_list_hidden = false;
                    this_clone.borrow().info_container.show();
                } else {
                    CONFIG.write().unwrap().ui.is_chapters_list_hidden = true;
                    this_clone.borrow().info_container.hide();
                }
            });
        this.show_chapters_btn.set_sensitive(true);

        // Draw thumnail image
        let this_clone = Rc::clone(this_rc);
        this.drawingarea
            .connect_draw(move |drawingarea, cairo_ctx| {
                let this = this_clone.borrow();
                this.draw_thumbnail(drawingarea, cairo_ctx)
            });

        // Scale seek
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.timeline_scale
            .connect_change_value(move |_, _, value| {
                main_ctrl_clone.borrow_mut().seek(value as u64, false); // approximate (fast)
                Inhibit(true)
            });

        // TreeView seek
        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.chapter_treeview
            .connect_row_activated(move |_, tree_path, _| {
                let position_opt = {
                    // get the position first in order to make sure
                    // this is no longer borrowed if main_ctrl::seek is to be called
                    let mut this = this_clone.borrow_mut();
                    match this.chapter_manager.get_iter(tree_path) {
                        Some(iter) => {
                            let position = this.chapter_manager.get_chapter_at_iter(&iter).start();
                            // update position
                            this.tick(position, false);
                            Some(position)
                        }
                        None => None,
                    }
                };

                if let Some(position) = position_opt {
                    main_ctrl_clone.borrow_mut().seek(position, true); // accurate (slow)
                }
            });

        // TreeView title modified
        if let Some(ref title_renderer) = this.chapter_manager.title_renderer {
            let this_clone = Rc::clone(this_rc);
            let main_ctrl_clone = Rc::clone(main_ctrl);
            title_renderer.connect_edited(move |_, _tree_path, new_title| {
                this_clone
                    .borrow_mut()
                    .chapter_manager
                    .rename_selected_chapter(new_title);
                // reflect title modification in the UI (audio waveform)
                main_ctrl_clone.borrow_mut().refresh();
            });
        }

        // Register add chapter action
        let add_chapter = gio::SimpleAction::new("add_chapter", None);
        gtk_app.add_action(&add_chapter);
        let this_clone = Rc::clone(this_rc);
        add_chapter.connect_activate(move |_, _| {
            this_clone.borrow_mut().add_chapter();
        });
        gtk_app.set_accels_for_action("app.add_chapter", &["plus", "KP_Add"]);

        // Register remove chapter action
        let remove_chapter = gio::SimpleAction::new("remove_chapter", None);
        gtk_app.add_action(&remove_chapter);
        let this_clone = Rc::clone(this_rc);
        remove_chapter.connect_activate(move |_, _| {
            this_clone.borrow_mut().remove_chapter();
        });
        gtk_app.set_accels_for_action("app.remove_chapter", &["minus", "KP_Subtract"]);

        // Register Toggle repeat current chapter action
        let toggle_repeat_chapter = gio::SimpleAction::new("toggle_repeat_chapter", None);
        gtk_app.add_action(&toggle_repeat_chapter);
        let repeat_btn = this.repeat_btn.clone();
        toggle_repeat_chapter.connect_activate(move |_, _| {
            repeat_btn.set_active(!repeat_btn.get_active());
        });
        gtk_app.set_accels_for_action("app.toggle_repeat_chapter", &["r"]);

        let this_clone = Rc::clone(this_rc);
        this.repeat_btn.connect_clicked(move |button| {
            this_clone.borrow_mut().repeat_chapter = button.get_active();
        });

        // Register next chapter action
        let next_chapter = gio::SimpleAction::new("next_chapter", None);
        gtk_app.add_action(&next_chapter);
        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        next_chapter.connect_activate(move |_, _| {
            let seek_pos = {
                let this = this_clone.borrow();
                this.chapter_manager
                    .next_iter()
                    .map(|next_iter| this.chapter_manager.get_chapter_at_iter(&next_iter).start())
            };

            if let Some(seek_pos) = seek_pos {
                main_ctrl_clone.borrow_mut().seek(seek_pos, true); // accurate (slow)
            }
        });
        gtk_app.set_accels_for_action("app.next_chapter", &["Down", "AudioNext"]);

        // Register previous chapter action
        let previous_chapter = gio::SimpleAction::new("previous_chapter", None);
        gtk_app.add_action(&previous_chapter);
        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        previous_chapter.connect_activate(move |_, _| {
            let seek_pos =
                {
                    let this = this_clone.borrow();
                    let position = this.get_position();
                    let cur_start = this.chapter_manager.get_selected_iter().map(|cur_iter| {
                        this.chapter_manager.get_chapter_at_iter(&cur_iter).start()
                    });
                    let prev_start = this.chapter_manager.prev_iter().map(|prev_iter| {
                        this.chapter_manager.get_chapter_at_iter(&prev_iter).start()
                    });

                    match (cur_start, prev_start) {
                        (Some(cur_start), prev_start_opt) => {
                            if cur_start + GO_TO_PREV_CHAPTER_THRESHOLD < position {
                                Some(cur_start)
                            } else {
                                prev_start_opt
                            }
                        }
                        (None, prev_start_opt) => prev_start_opt,
                    }
                }.unwrap_or(0);

            main_ctrl_clone.borrow_mut().seek(seek_pos, true); // accurate (slow)
        });
        gtk_app.set_accels_for_action("app.previous_chapter", &["Up", "AudioPrev"]);
    }

    fn draw_thumbnail(
        &self,
        drawingarea: &gtk::DrawingArea,
        cairo_ctx: &cairo::Context,
    ) -> Inhibit {
        // Thumbnail draw
        if let Some(ref surface) = self.thumbnail {
            let allocation = drawingarea.get_allocation();
            let alloc_ratio = f64::from(allocation.width) / f64::from(allocation.height);
            let surface_ratio = f64::from(surface.get_width()) / f64::from(surface.get_height());
            let scale = if surface_ratio < alloc_ratio {
                f64::from(allocation.height) / f64::from(surface.get_height())
            } else {
                f64::from(allocation.width) / f64::from(surface.get_width())
            };
            let x =
                (f64::from(allocation.width) / scale - f64::from(surface.get_width())).abs() / 2f64;
            let y = (f64::from(allocation.height) / scale - f64::from(surface.get_height())).abs()
                / 2f64;

            cairo_ctx.scale(scale, scale);
            cairo_ctx.set_source_surface(surface, x, y);
            cairo_ctx.paint();
        }

        Inhibit(true)
    }

    fn show_message(&self, message_type: gtk::MessageType, message: String) {
        let main_ctrl_weak = Weak::clone(self.main_ctrl.as_ref().unwrap());
        gtk::idle_add(move || {
            let main_ctrl_rc = main_ctrl_weak.upgrade().unwrap();
            main_ctrl_rc.borrow().show_message(message_type, &message);
            glib::Continue(false)
        });
    }

    fn show_error(&self, message: String) {
        self.show_message(gtk::MessageType::Error, message);
    }

    fn show_info(&self, message: String) {
        self.show_message(gtk::MessageType::Info, message);
    }

    pub fn new_media(&mut self, context: &PlaybackContext) {
        let toc_extensions = metadata::Factory::get_extensions();

        {
            let info = context.info.read().unwrap();

            // check the presence of a toc file
            let mut toc_candidates =
                toc_extensions
                    .into_iter()
                    .filter_map(|(extension, format)| {
                        let path = info
                            .path
                            .with_file_name(&format!("{}.{}", info.name, extension));
                        if path.is_file() {
                            Some((path, format))
                        } else {
                            None
                        }
                    });

            self.duration = info.duration;
            self.timeline_scale.set_range(0f64, info.duration as f64);
            self.duration_lbl
                .set_label(&Timestamp::format(info.duration, false));

            if let Some(ref image_sample) = info.get_image(0) {
                if let Some(ref image_buffer) = image_sample.get_buffer() {
                    if let Some(ref image_map) = image_buffer.map_readable() {
                        self.thumbnail =
                            ImageSurface::create_from_uknown(image_map.as_slice()).ok();
                    }
                }
            }

            self.title_lbl
                .set_label(info.get_title().unwrap_or(&EMPTY_REPLACEMENT));
            self.artist_lbl
                .set_label(info.get_artist().unwrap_or(&EMPTY_REPLACEMENT));
            self.container_lbl
                .set_label(info.get_container().unwrap_or(&EMPTY_REPLACEMENT));

            self.streams_changed(&info);

            let extern_toc = toc_candidates
                .next()
                .and_then(|(toc_path, format)| match File::open(toc_path.clone()) {
                    Ok(mut toc_file) => {
                        match metadata::Factory::get_reader(format).read(&info, &mut toc_file) {
                            Ok(Some(toc)) => Some(toc),
                            Ok(None) => {
                                let msg = gettext("No toc in file \"{}\"").replacen(
                                    "{}",
                                    toc_path.file_name().unwrap().to_str().unwrap(),
                                    1,
                                );
                                info!("{}", msg);
                                self.show_info(msg);
                                None
                            }
                            Err(err) => {
                                self.show_error(
                                    gettext("Error opening toc file \"{}\":\n{}")
                                        .replacen(
                                            "{}",
                                            toc_path.file_name().unwrap().to_str().unwrap(),
                                            1,
                                        ).replacen("{}", &err, 1),
                                );
                                None
                            }
                        }
                    }
                    Err(_) => {
                        self.show_error(gettext("Failed to open toc file."));
                        None
                    }
                });

            if extern_toc.is_some() {
                self.chapter_manager.replace_with(&extern_toc);
            } else {
                self.chapter_manager.replace_with(&info.toc);
            }
        }

        self.update_marks();

        self.repeat_btn.set_sensitive(true);
        self.add_chapter_btn.set_sensitive(true);
        match self.chapter_manager.get_selected_iter() {
            Some(current_iter) => {
                // position is in a chapter => select it
                self.chapter_treeview
                    .get_selection()
                    .select_iter(&current_iter);
                self.del_chapter_btn.set_sensitive(true);
            }
            None =>
            // position is not in any chapter
            {
                self.del_chapter_btn.set_sensitive(false)
            }
        }

        if self.thumbnail.is_some() {
            self.drawingarea.show();
            self.drawingarea.queue_draw();
        } else {
            self.drawingarea.hide();
        }
    }

    pub fn streams_changed(&self, info: &MediaInfo) {
        self.audio_codec_lbl
            .set_label(info.get_audio_codec().unwrap_or(&EMPTY_REPLACEMENT));
        self.video_codec_lbl
            .set_label(info.get_video_codec().unwrap_or(&EMPTY_REPLACEMENT));
    }

    fn update_marks(&self) {
        self.timeline_scale.clear_marks();

        let timeline_scale = self.timeline_scale.clone();
        self.chapter_manager.for_each(None, move |chapter| {
            timeline_scale.add_mark(chapter.start() as f64, gtk::PositionType::Top, None);
            true // keep going until the last chapter
        });
    }

    pub fn cleanup(&mut self) {
        self.title_lbl.set_text("");
        self.artist_lbl.set_text("");
        self.container_lbl.set_text("");
        self.audio_codec_lbl.set_text("");
        self.video_codec_lbl.set_text("");
        self.duration_lbl.set_text("00:00.000");
        self.thumbnail = None;
        self.chapter_manager.clear();
        self.add_chapter_btn.set_sensitive(false);
        self.del_chapter_btn.set_sensitive(false);
        self.timeline_scale.clear_marks();
        self.timeline_scale.set_value(0f64);
        self.duration = 0;
    }

    fn repeat_at(main_ctrl: &Option<Weak<RefCell<MainController>>>, position: u64) {
        let main_ctrl_weak = Weak::clone(main_ctrl.as_ref().unwrap());
        gtk::idle_add(move || {
            let main_ctrl_rc = main_ctrl_weak.upgrade().unwrap();
            main_ctrl_rc.borrow_mut().seek(position, true); // accurate (slow)
            glib::Continue(false)
        });
    }

    pub fn tick(&mut self, position: u64, is_eos: bool) {
        self.timeline_scale.set_value(position as f64);

        let (mut has_changed, prev_selected_iter) = self.chapter_manager.update_position(position);

        if self.repeat_chapter {
            // repeat is activated
            if is_eos {
                // postpone chapter selection change until media as synchronized
                has_changed = false;
                self.chapter_manager.rewind();
                InfoController::repeat_at(&self.main_ctrl, 0);
            } else if has_changed {
                if let Some(ref prev_selected_iter) = prev_selected_iter {
                    // discard has_changed because we will be looping on current chapter
                    has_changed = false;

                    // unselect chapter in order to avoid tracing change to current position
                    self.chapter_manager.unselect();
                    InfoController::repeat_at(
                        &self.main_ctrl,
                        self.chapter_manager
                            .get_chapter_at_iter(prev_selected_iter)
                            .start(),
                    );
                }
            }
        }

        if has_changed {
            // chapter has changed
            match self.chapter_manager.get_selected_iter() {
                Some(current_iter) => {
                    // position is in a chapter => select it
                    self.chapter_treeview
                        .get_selection()
                        .select_iter(&current_iter);
                    self.del_chapter_btn.set_sensitive(true);
                }
                None =>
                // position is not in any chapter
                {
                    if let Some(ref prev_selected_iter) = prev_selected_iter {
                        // but a previous chapter was selected => unselect it
                        self.chapter_treeview
                            .get_selection()
                            .unselect_iter(prev_selected_iter);
                        self.del_chapter_btn.set_sensitive(false);
                    }
                }
            }
        }
    }

    pub fn move_chapter_boundary(&mut self, boundary: u64, to_position: u64) -> bool {
        self.chapter_manager
            .move_chapter_boundary(boundary, to_position)
    }

    pub fn seek(&mut self, position: u64, state: &ControllerState) {
        self.chapter_manager.prepare_for_seek();

        if *state != ControllerState::Playing {
            // force sync
            self.tick(position, false);
        }
    }

    pub fn start_play_range(&mut self) {
        self.chapter_manager.prepare_for_seek();
    }

    fn get_position(&self) -> u64 {
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        let mut main_ctrl = main_ctrl_rc.borrow_mut();
        main_ctrl.get_position()
    }

    pub fn add_chapter(&mut self) {
        let position = self.get_position();
        if position >= self.duration {
            // can't add a chapter starting at last position
            return;
        }

        if let Some(new_iter) = self.chapter_manager.add_chapter(position, self.duration) {
            self.chapter_treeview.get_selection().select_iter(&new_iter);
            self.update_marks();
            self.del_chapter_btn.set_sensitive(true);
        }
    }

    pub fn remove_chapter(&mut self) {
        match self.chapter_manager.remove_selected_chapter() {
            Some(new_iter) => self.chapter_treeview.get_selection().select_iter(&new_iter),
            None => {
                self.chapter_treeview.get_selection().unselect_all();
                self.del_chapter_btn.set_sensitive(false);
            }
        }

        self.update_marks();
    }

    pub fn export_chapters(&self, context: &mut PlaybackContext) {
        if let Some((toc, count)) = self.chapter_manager.get_toc() {
            let mut info = context.info.write().unwrap();
            info.toc = Some(toc);
            info.chapter_count = Some(count);
        }
    }
}
