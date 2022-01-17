use gettextrs::gettext;
use gtk::{cairo, gio, glib, prelude::*};
use log::{info, warn};

use std::{cell::RefCell, fs::File, rc::Rc};

use application::CONFIG;
use media::pipeline;
use metadata::{Duration, MediaInfo, Timestamp4Humans};
use renderers::{Image, Timestamp};

use super::{ChapterTreeManager, ChaptersBoundaries, PositionStatus};
use crate::{info_bar, main, playback, prelude::*};

const EMPTY_REPLACEMENT: &str = "-";
const GO_TO_PREV_CHAPTER_THRESHOLD: Duration = Duration::from_secs(1);

pub struct Controller {
    pub(super) info_container: gtk::Grid,
    pub(super) show_chapters_btn: gtk::ToggleButton,

    pub(super) drawingarea: gtk::DrawingArea,

    title_lbl: gtk::Label,
    artist_lbl: gtk::Label,
    container_lbl: gtk::Label,
    audio_codec_lbl: gtk::Label,
    video_codec_lbl: gtk::Label,
    duration_lbl: gtk::Label,

    pub(super) timeline_scale: gtk::Scale,
    pub(super) repeat_btn: gtk::ToggleToolButton,

    pub(super) chapter_treeview: gtk::TreeView,
    add_chapter_btn: gtk::ToolButton,
    pub(super) add_chapter_action: gio::SimpleAction,
    del_chapter_btn: gtk::ToolButton,
    pub(super) del_chapter_action: gio::SimpleAction,

    pub(super) next_chapter_action: gio::SimpleAction,
    pub(super) previous_chapter_action: gio::SimpleAction,

    thumbnail_handler: Option<glib::SignalHandlerId>,

    pub(crate) chapter_manager: ChapterTreeManager,

    duration: Duration,
    pub(crate) repeat_chapter: bool,
}

impl UIController for Controller {
    fn new_media(&mut self, pipeline: &pipeline::Playback) {
        let toc_extensions = metadata::Factory::extensions();

        {
            let info = pipeline.info.read().unwrap();

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
            self.timeline_scale.set_range(0f64, info.duration.as_f64());
            self.duration_lbl
                .set_label(&Timestamp4Humans::from_duration(info.duration).to_string());

            let mut thumbnail = info.media_image().and_then(|image| {
                image.buffer().and_then(|image_buffer| {
                    image_buffer.map_readable().ok().and_then(|image_map| {
                        Image::from_unknown(image_map.as_slice())
                            .map_err(|err| warn!("{}", err))
                            .ok()
                    })
                })
            });

            if let Some(thumbnail) = thumbnail.take() {
                self.thumbnail_handler = Some(self.drawingarea.connect_draw(
                    move |drawingarea, cairo_ctx| {
                        Self::draw_thumbnail(&thumbnail, drawingarea, cairo_ctx);
                        Inhibit(false)
                    },
                ));
            }

            self.container_lbl
                .set_label(info.container().unwrap_or(EMPTY_REPLACEMENT));

            let extern_toc = toc_candidates
                .next()
                .and_then(|(toc_path, format)| match File::open(toc_path.clone()) {
                    Ok(mut toc_file) => {
                        match metadata::Factory::reader(format).read(&info, &mut toc_file) {
                            Ok(Some(toc)) => Some(toc),
                            Ok(None) => {
                                let msg = gettext("No toc in file \"{}\"").replacen(
                                    "{}",
                                    toc_path.file_name().unwrap().to_str().unwrap(),
                                    1,
                                );
                                info!("{}", msg);
                                info_bar::show_info(msg);
                                None
                            }
                            Err(err) => {
                                info_bar::show_error(
                                    gettext("Error opening toc file \"{}\":\n{}")
                                        .replacen(
                                            "{}",
                                            toc_path.file_name().unwrap().to_str().unwrap(),
                                            1,
                                        )
                                        .replacen("{}", &err, 1),
                                );
                                None
                            }
                        }
                    }
                    Err(_) => {
                        info_bar::show_error(gettext("Failed to open toc file."));
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
        self.add_chapter_action.set_enabled(true);
        match self.chapter_manager.selected() {
            Some(sel_chapter) => {
                // position is in a chapter => select it
                self.chapter_treeview
                    .selection()
                    .select_iter(sel_chapter.iter());
                self.del_chapter_btn.set_sensitive(true);
                self.del_chapter_action.set_enabled(true);
            }
            None =>
            // position is not in any chapter
            {
                self.del_chapter_btn.set_sensitive(false);
                self.del_chapter_action.set_enabled(false);
            }
        }

        self.next_chapter_action.set_enabled(true);
        self.previous_chapter_action.set_enabled(true);

        if self.thumbnail_handler.is_some() {
            self.drawingarea.show();
            self.drawingarea.queue_draw();
        } else {
            self.drawingarea.hide();
        }

        main::update_focus();
    }

    fn cleanup(&mut self) {
        self.title_lbl.set_text("");
        self.artist_lbl.set_text("");
        self.container_lbl.set_text("");
        self.audio_codec_lbl.set_text("");
        self.video_codec_lbl.set_text("");
        self.duration_lbl.set_text("00:00.000");
        if let Some(thumbnail_handler) = self.thumbnail_handler.take() {
            glib::signal_handler_disconnect(&self.drawingarea, thumbnail_handler);
        }
        self.chapter_treeview.selection().unselect_all();
        self.chapter_manager.clear();
        self.add_chapter_btn.set_sensitive(false);
        self.add_chapter_action.set_enabled(false);
        self.del_chapter_btn.set_sensitive(false);
        self.del_chapter_action.set_enabled(false);
        self.next_chapter_action.set_enabled(false);
        self.previous_chapter_action.set_enabled(false);
        self.timeline_scale.clear_marks();
        self.timeline_scale.set_value(0f64);
        self.duration = Duration::default();
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        match info.media_artist() {
            Some(artist) => self.artist_lbl.set_label(&artist),
            None => self.artist_lbl.set_label(EMPTY_REPLACEMENT),
        }
        match info.media_title() {
            Some(title) => self.title_lbl.set_label(&title),
            None => self.title_lbl.set_label(EMPTY_REPLACEMENT),
        }

        self.audio_codec_lbl
            .set_label(info.streams.audio_codec().unwrap_or(EMPTY_REPLACEMENT));
        self.video_codec_lbl
            .set_label(info.streams.video_codec().unwrap_or(EMPTY_REPLACEMENT));
    }

    fn grab_focus(&self) {
        self.chapter_treeview.grab_focus();

        match self.chapter_manager.selected_path() {
            Some(sel_path) => {
                self.chapter_treeview
                    .set_cursor(&sel_path, None::<&gtk::TreeViewColumn>, false);
                self.chapter_treeview.grab_default();

                self.repeat_btn.set_sensitive(true);

                self.del_chapter_btn.set_sensitive(true);
                self.del_chapter_action.set_enabled(true);
            }
            None => {
                // Set the cursor to an uninitialized path to unselect
                self.chapter_treeview.set_cursor(
                    &gtk::TreePath::new(),
                    None::<&gtk::TreeViewColumn>,
                    false,
                );

                self.del_chapter_btn.set_sensitive(false);
                self.del_chapter_action.set_enabled(false);
            }
        }

        self.add_chapter_btn.set_sensitive(true);
        self.add_chapter_action.set_enabled(true);

        self.chapter_treeview.set_sensitive(true);
    }
}

impl Controller {
    pub fn new(builder: &gtk::Builder, boundaries: Rc<RefCell<ChaptersBoundaries>>) -> Self {
        let mut chapter_manager =
            ChapterTreeManager::new(builder.object("chapters-tree-store").unwrap(), boundaries);
        let chapter_treeview: gtk::TreeView = builder.object("chapter-treeview").unwrap();
        chapter_manager.init_treeview(&chapter_treeview);

        let mut ctrl = Controller {
            info_container: builder.object("info-chapter_list-grid").unwrap(),
            show_chapters_btn: builder.object("show_chapters-toggle").unwrap(),

            drawingarea: builder.object("thumbnail-drawingarea").unwrap(),

            title_lbl: builder.object("title-lbl").unwrap(),
            artist_lbl: builder.object("artist-lbl").unwrap(),
            container_lbl: builder.object("container-lbl").unwrap(),
            audio_codec_lbl: builder.object("audio_codec-lbl").unwrap(),
            video_codec_lbl: builder.object("video_codec-lbl").unwrap(),
            duration_lbl: builder.object("duration-lbl").unwrap(),

            timeline_scale: builder.object("timeline-scale").unwrap(),
            repeat_btn: builder.object("repeat-toolbutton").unwrap(),

            chapter_treeview,
            add_chapter_btn: builder.object("add_chapter-toolbutton").unwrap(),
            add_chapter_action: gio::SimpleAction::new("add_chapter", None),
            del_chapter_btn: builder.object("del_chapter-toolbutton").unwrap(),
            del_chapter_action: gio::SimpleAction::new("del_chapter", None),

            next_chapter_action: gio::SimpleAction::new("next_chapter", None),
            previous_chapter_action: gio::SimpleAction::new("previous_chapter", None),

            thumbnail_handler: None,

            chapter_manager,

            duration: Duration::default(),
            repeat_chapter: false,
        };

        ctrl.cleanup();

        // Show chapters toggle
        if CONFIG.read().unwrap().ui.is_chapters_list_hidden {
            ctrl.show_chapters_btn.set_active(false);
            ctrl.info_container.hide();
        }

        ctrl.show_chapters_btn.set_sensitive(true);

        ctrl
    }

    pub fn loose_focus(&self) {
        self.chapter_treeview.set_sensitive(false);

        self.add_chapter_btn.set_sensitive(false);
        self.add_chapter_action.set_enabled(false);
        self.del_chapter_btn.set_sensitive(false);
        self.del_chapter_action.set_enabled(false);
    }

    pub fn draw_thumbnail(
        thumbnail: &Image,
        drawingarea: &gtk::DrawingArea,
        cairo_ctx: &cairo::Context,
    ) {
        let allocation = drawingarea.allocation();
        let alloc_width_f: f64 = allocation.width().into();
        let alloc_height_f: f64 = allocation.height().into();

        let image_width_f: f64 = thumbnail.width().into();
        let image_height_f: f64 = thumbnail.height().into();

        let alloc_ratio = alloc_width_f / alloc_height_f;
        let image_ratio = image_width_f / image_height_f;
        let scale = if image_ratio < alloc_ratio {
            alloc_height_f / image_height_f
        } else {
            alloc_width_f / image_width_f
        };
        let x = (alloc_width_f / scale - image_width_f).abs() / 2f64;
        let y = (alloc_height_f / scale - image_height_f).abs() / 2f64;

        thumbnail.with_surface_external_context(cairo_ctx, |cr, surface| {
            cr.scale(scale, scale);
            cr.set_source_surface(surface, x, y).unwrap();
            cr.paint().unwrap();
        })
    }

    fn update_marks(&self) {
        self.timeline_scale.clear_marks();

        let timeline_scale = self.timeline_scale.clone();
        self.chapter_manager.iter().for_each(move |chapter| {
            timeline_scale.add_mark(chapter.start().as_f64(), gtk::PositionType::Top, None);
        });
    }

    fn repeat_at(&self, ts: Timestamp) {
        playback::seek(ts, gst::SeekFlags::ACCURATE)
    }

    pub fn tick(&mut self, ts: Timestamp, state: main::State) {
        self.timeline_scale.set_value(ts.as_f64());

        let mut position_status = self.chapter_manager.update_ts(ts);

        if self.repeat_chapter {
            // repeat is activated
            if state.is_eos() {
                // postpone chapter selection change until media has synchronized
                position_status = PositionStatus::ChapterNotChanged;
                self.repeat_at(Timestamp::default());
            } else if let PositionStatus::ChapterChanged {
                prev_chapter: Some(prev_chapter),
            } = &position_status
            {
                // reset position_status because we will be looping on current chapter
                let prev_start = prev_chapter.start;
                position_status = PositionStatus::ChapterNotChanged;

                // unselect chapter in order to avoid tracing change to current timestamp
                self.chapter_manager.unselect();
                self.repeat_at(prev_start);
            }
        }

        if let PositionStatus::ChapterChanged { prev_chapter } = position_status {
            // let go the mutable reference on `self.chapter_manager`
            match self.chapter_manager.selected() {
                Some(sel_chapter) => {
                    // timestamp is in a chapter => select it
                    self.chapter_treeview
                        .selection()
                        .select_iter(sel_chapter.iter());
                    self.del_chapter_btn.set_sensitive(true);
                    self.del_chapter_action.set_enabled(true);
                }
                None =>
                // timestamp is not in any chapter
                {
                    if let Some(prev_chapter) = prev_chapter {
                        // but a previous chapter was selected => unselect it
                        self.chapter_treeview
                            .selection()
                            .unselect_iter(&prev_chapter.iter);
                        self.del_chapter_btn.set_sensitive(false);
                        self.del_chapter_action.set_enabled(false);
                    }
                }
            }

            main::update_focus();
        }
    }

    pub fn move_chapter_boundary(
        &mut self,
        boundary: Timestamp,
        target: Timestamp,
    ) -> PositionStatus {
        self.chapter_manager.move_chapter_boundary(boundary, target)
    }

    pub fn add_chapter(&mut self, ts: Timestamp) {
        if ts >= self.duration {
            // can't add a chapter starting at last position
            return;
        }

        if let Some(new_iter) = self.chapter_manager.add_chapter(ts, self.duration) {
            self.chapter_treeview.selection().select_iter(&new_iter);
            self.update_marks();
            self.del_chapter_btn.set_sensitive(true);
            self.del_chapter_action.set_enabled(true);
        }
    }

    pub fn remove_chapter(&mut self) {
        match self.chapter_manager.remove_selected_chapter() {
            Some(new_iter) => self.chapter_treeview.selection().select_iter(&new_iter),
            None => {
                self.chapter_treeview.selection().unselect_all();
                self.del_chapter_btn.set_sensitive(false);
                self.del_chapter_action.set_enabled(false);
            }
        }

        self.update_marks();
    }

    pub fn export_chapters(&self, info: &mut MediaInfo) {
        if let Some((toc, count)) = self.chapter_manager.toc() {
            info.toc = Some(toc);
            info.chapter_count = Some(count);
        }
    }

    pub fn toggle_chapter_list(&self, must_show: bool) {
        CONFIG.write().unwrap().ui.is_chapters_list_hidden = must_show;

        if must_show {
            self.info_container.hide();
        } else {
            self.info_container.show();
        }
    }

    pub fn previous_chapter(&self, cur_ts: Timestamp) -> Option<Timestamp> {
        let cur_start = self
            .chapter_manager
            .selected()
            .map(|sel_chapter| sel_chapter.start());
        let prev_start = self
            .chapter_manager
            .pick_previous()
            .map(|prev_chapter| prev_chapter.start());

        match (cur_start, prev_start) {
            (Some(cur_start), prev_start_opt) => {
                if cur_ts > cur_start + GO_TO_PREV_CHAPTER_THRESHOLD {
                    Some(cur_start)
                } else {
                    prev_start_opt
                }
            }
            (None, prev_start_opt) => prev_start_opt,
        }
    }
}
