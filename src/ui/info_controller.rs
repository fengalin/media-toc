extern crate cairo;

extern crate gtk;
use gtk::prelude::*;

extern crate glib;

use std::rc::{Rc, Weak};
use std::cell::RefCell;

use media::{Chapter, Context, Timestamp};

use super::{ChapterTreeManager, ControllerState, ImageSurface, MainController};

pub struct InfoController {
    drawingarea: gtk::DrawingArea,

    title_lbl: gtk::Label,
    artist_lbl: gtk::Label,
    container_lbl: gtk::Label,
    audio_codec_lbl: gtk::Label,
    video_codec_lbl: gtk::Label,
    duration_lbl: gtk::Label,

    timeline_scale: gtk::Scale,
    repeat_button: gtk::ToggleToolButton,

    chapter_treeview: gtk::TreeView,
    add_chapter_btn: gtk::ToolButton,
    del_chapter_btn: gtk::ToolButton,

    thumbnail: Option<ImageSurface>,

    chapter_manager: ChapterTreeManager,

    duration: u64,
    repeat_chapter: bool,

    main_ctrl: Option<Weak<RefCell<MainController>>>,
}

impl InfoController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let add_chapter_btn: gtk::ToolButton = builder
            .get_object("add_chapter-toolbutton")
            .unwrap();
        add_chapter_btn.set_sensitive(false);
        let del_chapter_btn: gtk::ToolButton = builder
            .get_object("remove_chapter-toolbutton")
            .unwrap();
        del_chapter_btn.set_sensitive(false);

        let chapter_manager = ChapterTreeManager::new_from(
            builder.get_object("chapters-tree-store").unwrap()
        );
        let chapter_treeview: gtk::TreeView = builder.get_object("chapter-treeview").unwrap();
        chapter_manager.init_treeview(&chapter_treeview);

        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let this_rc = Rc::new(RefCell::new(InfoController {
            drawingarea: builder.get_object("thumbnail-drawingarea").unwrap(),

            title_lbl: builder.get_object("title-lbl").unwrap(),
            artist_lbl: builder.get_object("artist-lbl").unwrap(),
            container_lbl: builder.get_object("container-lbl").unwrap(),
            audio_codec_lbl: builder.get_object("audio_codec-lbl").unwrap(),
            video_codec_lbl: builder.get_object("video_codec-lbl").unwrap(),
            duration_lbl: builder.get_object("duration-lbl").unwrap(),

            timeline_scale: builder.get_object("timeline-scale").unwrap(),
            repeat_button: builder.get_object("repeat-toolbutton").unwrap(),

            chapter_treeview: chapter_treeview,
            add_chapter_btn: add_chapter_btn,
            del_chapter_btn: del_chapter_btn,

            thumbnail: None,

            chapter_manager: chapter_manager,

            duration: 0,
            repeat_chapter: false,

            main_ctrl: None,
        }));

        {
            let mut this = this_rc.borrow_mut();
            this.cleanup();
            this.chapter_treeview.set_activate_on_single_click(true);
        }

        this_rc
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();

        this.main_ctrl = Some(Rc::downgrade(main_ctrl));

        // Draw thumnail image
        let this_clone = Rc::clone(this_rc);
        this.drawingarea.connect_draw(
            move |drawingarea, cairo_ctx| {
                let this = this_clone.borrow();
                this.draw_thumbnail(drawingarea, cairo_ctx)
            },
        );

        // Scale seek
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.timeline_scale.connect_change_value(
            move |_, _, value| {
                main_ctrl_clone.borrow_mut().seek(value as u64, false); // approximate (fast)
                Inhibit(true)
            },
        );

        // TreeView seek
        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.chapter_treeview.connect_row_activated(
            move |_, tree_path, _| {
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
            },
        );

        // add chapter
        let this_clone = Rc::clone(this_rc);
        this.add_chapter_btn.connect_clicked(move |_| {
            this_clone.borrow_mut().add_chapter();
        });

        // remove chapter
        let this_clone = Rc::clone(this_rc);
        this.del_chapter_btn.connect_clicked(move |_| {
            this_clone.borrow_mut().remove_chapter();
        });

        // repeat button
        let this_clone = Rc::clone(this_rc);
        this.repeat_button.connect_clicked(move |button| {
            this_clone.borrow_mut().repeat_chapter = button.get_active();
        });
    }

    fn draw_thumbnail(
        &self,
        drawingarea: &gtk::DrawingArea,
        cairo_ctx: &cairo::Context,
    ) -> Inhibit {
        // Thumbnail draw
        if let Some(ref thumbnail) = self.thumbnail {
            let surface = &thumbnail.surface;

            let allocation = drawingarea.get_allocation();
            let alloc_ratio = f64::from(allocation.width) / f64::from(allocation.height);
            let surface_ratio = f64::from(surface.get_width()) / f64::from(surface.get_height());
            let scale = if surface_ratio < alloc_ratio {
                f64::from(allocation.height) / f64::from(surface.get_height())
            } else {
                f64::from(allocation.width) / f64::from(surface.get_width())
            };
            let x = (f64::from(allocation.width) / scale - f64::from(surface.get_width())).abs() /
                2f64;
            let y = (f64::from(allocation.height) / scale - f64::from(surface.get_height()))
                .abs() / 2f64;

            cairo_ctx.scale(scale, scale);
            cairo_ctx.set_source_surface(surface, x, y);
            cairo_ctx.paint();
        }

        Inhibit(true)
    }

    pub fn new_media(&mut self, context: &Context) {
        self.update_duration(context.get_duration());

        {
            let mut info = context.info.lock().expect(
                "InfoController::new_media failed to lock media info",
            );

            info.fix();

            if let Some(thumbnail) = info.thumbnail.take() {
                if let Ok(image) = ImageSurface::from_aligned_image(thumbnail) {
                    self.thumbnail = Some(image);
                }
            };

            self.title_lbl.set_label(&info.title);
            self.artist_lbl.set_label(&info.artist);
            self.container_lbl.set_label(if !info.container.is_empty() {
                &info.container
            } else {
                "-"
            });
            self.audio_codec_lbl.set_label(
                if !info.audio_codec.is_empty() {
                    &info.audio_codec
                } else {
                    "-"
                },
            );
            self.video_codec_lbl.set_label(
                if !info.video_codec.is_empty() {
                    &info.video_codec
                } else {
                    "-"
                },
            );

            self.chapter_manager.replace_with(&info.chapters);

            self.update_marks();
        }

        self.add_chapter_btn.set_sensitive(true);
        match self.chapter_manager.get_selected_iter() {
            Some(current_iter) => {
                // position is in a chapter => select it
                self.chapter_treeview.get_selection().select_iter(&current_iter);
                self.del_chapter_btn.set_sensitive(true);
            }
            None =>
                // position is not in any chapter
                self.del_chapter_btn.set_sensitive(false),
        }

        if self.thumbnail.is_some() {
            self.drawingarea.show();
            self.drawingarea.queue_draw();
        } else {
            self.drawingarea.hide();
        }
    }

    fn update_marks(&self) {
        self.timeline_scale.clear_marks();

        let timeline_scale = self.timeline_scale.clone();
        self.chapter_manager.for_each(None, move |chapter| {
            timeline_scale.add_mark(
                chapter.start() as f64,
                gtk::PositionType::Top,
                None,
            );
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
        self.timeline_scale.clear_marks();
        self.timeline_scale.set_value(0f64);
        self.duration = 0;
    }

    pub fn update_duration(&mut self, duration: u64) {
        self.duration = duration;
        self.timeline_scale.set_range(0f64, duration as f64);
        self.duration_lbl.set_label(
            &Timestamp::format(duration, false),
        );
    }

    fn repeat_at(main_ctrl: &Option<Weak<RefCell<MainController>>>, position: u64) {
        let main_ctrl_weak = Weak::clone(main_ctrl.as_ref().unwrap());
        gtk::idle_add(move || {
            let main_ctrl_rc = main_ctrl_weak.upgrade().expect(
                "InfoController::tick can't upgrade main_ctrl while repeating chapter",
            );
            main_ctrl_rc.borrow_mut().seek(position, true); // accurate (slow)
            glib::Continue(false)
        });
    }

    pub fn tick(&mut self, position: u64, is_eos: bool) {
        self.timeline_scale.set_value(position as f64);

        let (mut has_changed, prev_selected_iter) =
            self.chapter_manager.update_position(position);

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
                    InfoController::repeat_at(&self.main_ctrl,
                        self.chapter_manager.get_chapter_at_iter(prev_selected_iter).start()
                    );
                }
            }
        }

        if has_changed {
            // chapter has changed
            match self.chapter_manager.get_selected_iter() {
                Some(current_iter) => {
                    // position is in a chapter => select it
                    self.chapter_treeview.get_selection().select_iter(&current_iter);
                    self.del_chapter_btn.set_sensitive(true);
                }
                None =>
                    // position is not in any chapter
                    if let Some(ref prev_selected_iter) = prev_selected_iter {
                        // but a previous chapter was selected => unselect it
                        self.chapter_treeview.get_selection().unselect_iter(prev_selected_iter);
                        self.del_chapter_btn.set_sensitive(false);
                    },
            }
        }
    }

    pub fn seek(&mut self, position: u64, state: &ControllerState) {
        self.chapter_manager.prepare_for_seek();

        if *state == ControllerState::Paused {
            // force sync
            self.tick(position, false);
        }
    }

    fn get_position(&self) -> u64 {
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().expect(
            "InfoController::get_position can't upgrade main_ctrl",
        );
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
            Some(new_iter) =>
                self.chapter_treeview.get_selection().select_iter(&new_iter),
            None => {
                self.chapter_treeview.get_selection().unselect_all();
                self.del_chapter_btn.set_sensitive(false);
            }
        }

        self.update_marks();
    }

    pub fn export_chapters(&self, context: &mut Context) {
        let mut chapters = Vec::<Chapter>::new();
        self.chapter_manager.for_each(None, |chapter| {
            chapters.push(Chapter::new(
                // FIXME: really use an id
                &chapter.start_str(),
                &chapter.title(),
                chapter.start_ts(),
                chapter.end_ts(),
            ));
            true // keep going until the last chapter
        });

        let mut info = context.info.lock().expect(
            "InfoController::export_chapters failed to lock media info",
        );
        info.chapters = chapters;
    }
}
