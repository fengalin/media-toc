use cairo;
use gettextrs::gettext;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;
use lazy_static::lazy_static;
use log::{info, warn};

use std::{cell::RefCell, fs::File, rc::Rc};

use media::Timestamp;
use metadata;
use metadata::{MediaInfo, Timestamp4Humans};
use renderers::Image;

use super::{
    ChapterTreeManager, ChaptersBoundaries, ControllerState, PlaybackPipeline, PositionStatus,
    UIController, UIEventSender,
};
use crate::application::{CommandLineArguments, CONFIG};

lazy_static! {
    static ref EMPTY_REPLACEMENT: &'static str = "-";
}

pub struct InfoController {
    ui_event: UIEventSender,

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
    del_chapter_btn: gtk::ToolButton,

    thumbnail: Option<Image>,

    pub(super) chapter_manager: ChapterTreeManager,

    // FIXME: use a Duration struct
    duration: u64,
    pub(super) repeat_chapter: bool,
}

impl UIController for InfoController {
    fn setup(&mut self, _args: &CommandLineArguments) {
        self.cleanup();

        // Show chapters toggle
        if CONFIG.read().unwrap().ui.is_chapters_list_hidden {
            self.show_chapters_btn.set_active(false);
            self.info_container.hide();
        }

        self.show_chapters_btn.set_sensitive(true);
    }

    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let toc_extensions = metadata::Factory::get_extensions();

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
            self.timeline_scale.set_range(0f64, info.duration as f64);
            self.duration_lbl
                .set_label(&Timestamp4Humans::from_nano(info.duration).to_string());

            self.thumbnail = info.get_media_image().and_then(|image| {
                image.get_buffer().and_then(|image_buffer| {
                    image_buffer.map_readable().and_then(|image_map| {
                        Image::from_unknown(image_map.as_slice())
                            .map_err(|err| warn!("{}", err))
                            .ok()
                    })
                })
            });

            self.container_lbl
                .set_label(info.get_container().unwrap_or(&EMPTY_REPLACEMENT));

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
                                self.ui_event.show_info(msg);
                                None
                            }
                            Err(err) => {
                                self.ui_event.show_error(
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
                        self.ui_event
                            .show_error(gettext("Failed to open toc file."));
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
        match self.chapter_manager.selected() {
            Some(sel_chapter) => {
                // position is in a chapter => select it
                self.chapter_treeview
                    .get_selection()
                    .select_iter(sel_chapter.iter());
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

    fn cleanup(&mut self) {
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

    fn streams_changed(&mut self, info: &MediaInfo) {
        match info.get_media_artist() {
            Some(artist) => self.artist_lbl.set_label(&artist),
            None => self.artist_lbl.set_label(&EMPTY_REPLACEMENT),
        }
        match info.get_media_title() {
            Some(title) => self.title_lbl.set_label(&title),
            None => self.title_lbl.set_label(&EMPTY_REPLACEMENT),
        }

        self.audio_codec_lbl
            .set_label(info.streams.get_audio_codec().unwrap_or(&EMPTY_REPLACEMENT));
        self.video_codec_lbl
            .set_label(info.streams.get_video_codec().unwrap_or(&EMPTY_REPLACEMENT));
    }
}

impl InfoController {
    pub fn new(
        builder: &gtk::Builder,
        ui_event_sender: UIEventSender,
        boundaries: Rc<RefCell<ChaptersBoundaries>>,
    ) -> Self {
        let mut chapter_manager = ChapterTreeManager::new(
            builder.get_object("chapters-tree-store").unwrap(),
            boundaries,
        );
        let chapter_treeview: gtk::TreeView = builder.get_object("chapter-treeview").unwrap();
        chapter_manager.init_treeview(&chapter_treeview);

        InfoController {
            ui_event: ui_event_sender,

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
        }
    }

    pub fn draw_thumbnail(&mut self, drawingarea: &gtk::DrawingArea, cairo_ctx: &cairo::Context) {
        if let Some(image) = self.thumbnail.as_mut() {
            let allocation = drawingarea.get_allocation();
            let alloc_width_f: f64 = allocation.width.into();
            let alloc_height_f: f64 = allocation.height.into();

            let image_width_f: f64 = image.width().into();
            let image_height_f: f64 = image.height().into();

            let alloc_ratio = alloc_width_f / alloc_height_f;
            let image_ratio = image_width_f / image_height_f;
            let scale = if image_ratio < alloc_ratio {
                alloc_height_f / image_height_f
            } else {
                alloc_width_f / image_width_f
            };
            let x = (alloc_width_f / scale - image_width_f).abs() / 2f64;
            let y = (alloc_height_f / scale - image_height_f).abs() / 2f64;

            image.with_surface_external_context(cairo_ctx, |cr, surface| {
                cr.scale(scale, scale);
                cr.set_source_surface(surface, x, y);
                cr.paint();
            })
        }
    }

    fn update_marks(&self) {
        self.timeline_scale.clear_marks();

        let timeline_scale = self.timeline_scale.clone();
        self.chapter_manager.iter().for_each(move |chapter| {
            timeline_scale.add_mark(chapter.start().as_f64(), gtk::PositionType::Top, None);
        });
    }

    fn repeat_at(&self, ts: Timestamp) {
        self.ui_event.seek(ts, gst::SeekFlags::ACCURATE)
    }

    pub fn tick(&mut self, ts: Timestamp, state: ControllerState) {
        self.timeline_scale.set_value(ts.as_f64());

        let mut position_status = self.chapter_manager.update_ts(ts);

        if self.repeat_chapter {
            // repeat is activated
            if state == ControllerState::EOS {
                // postpone chapter selection change until media has synchronized
                position_status = PositionStatus::ChapterNotChanged;
                self.repeat_at(Timestamp::default());
            } else if let PositionStatus::ChapterChanged { prev_chapter } = &position_status {
                if let Some(prev_chapter) = prev_chapter {
                    // reset position_status because we will be looping on current chapter
                    let prev_start = prev_chapter.start;
                    position_status = PositionStatus::ChapterNotChanged;

                    // unselect chapter in order to avoid tracing change to current timestamp
                    self.chapter_manager.unselect();
                    self.repeat_at(prev_start);
                }
            }
        }

        if let PositionStatus::ChapterChanged { prev_chapter } = position_status {
            // let go the mutable reference on `self.chapter_manager`
            match self.chapter_manager.selected() {
                Some(sel_chapter) => {
                    // timestamp is in a chapter => select it
                    self.chapter_treeview
                        .get_selection()
                        .select_iter(sel_chapter.iter());
                    self.del_chapter_btn.set_sensitive(true);
                }
                None =>
                // timestamp is not in any chapter
                {
                    if let Some(prev_chapter) = prev_chapter {
                        // but a previous chapter was selected => unselect it
                        self.chapter_treeview
                            .get_selection()
                            .unselect_iter(&prev_chapter.iter);
                        self.del_chapter_btn.set_sensitive(false);
                    }
                }
            }
        }
    }

    pub fn move_chapter_boundary(
        &mut self,
        boundary: Timestamp,
        target: Timestamp,
    ) -> PositionStatus {
        self.chapter_manager.move_chapter_boundary(boundary, target)
    }

    pub fn seek(&mut self, target: Timestamp) {
        self.tick(target, ControllerState::Seeking);
    }

    pub fn add_chapter(&mut self, ts: Timestamp) {
        if ts.as_u64() >= self.duration {
            // can't add a chapter starting at last position
            return;
        }

        if let Some(new_iter) = self.chapter_manager.add_chapter(ts, self.duration) {
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

    pub fn export_chapters(&self, info: &mut MediaInfo) {
        if let Some((toc, count)) = self.chapter_manager.get_toc() {
            info.toc = Some(toc);
            info.chapter_count = Some(count);
        }
    }
}
