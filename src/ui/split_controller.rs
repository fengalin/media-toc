use gettextrs::gettext;
use glib;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;
use log::{error, warn};

use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
    path::PathBuf,
    rc::{Rc, Weak},
    sync::mpsc::{channel, Receiver},
};

use crate::{
    media::{PipelineMessage, PipelineMessage::*, PlaybackPipeline, SplitterPipeline},
    metadata,
    metadata::{get_default_chapter_title, Format, MediaContent, MediaInfo, Stream, TocVisitor},
};

use super::{MainController, OutputBaseController};

const LISTENER_PERIOD: u32 = 250; // 250 ms (4 Hz)

const TAGS_TO_SKIP: [&str; 12] = [
    "ApplicationName",
    "ApplicationData",
    "AudioCodec",
    "Codec",
    "ContainerFormat",
    "Duration",
    "Encoder",
    "EncoderVersion",
    "SubtitleCodec",
    "TrackCount",
    "TrackNumber",
    "VideoCodec",
];

pub struct SplitController {
    base: OutputBaseController,

    selected_audio: Option<Stream>,
    toc_visitor: Option<TocVisitor>,
    idx: usize,

    split_list: gtk::ListBox,
    split_to_flac_row: gtk::ListBoxRow,
    flac_warning_lbl: gtk::Label,
    split_to_wave_row: gtk::ListBoxRow,
    wave_warning_lbl: gtk::Label,
    split_to_opus_row: gtk::ListBoxRow,
    opus_warning_lbl: gtk::Label,
    split_to_vorbis_row: gtk::ListBoxRow,
    vorbis_warning_lbl: gtk::Label,
    split_to_mp3_row: gtk::ListBoxRow,
    mp3_warning_lbl: gtk::Label,
    split_progress_bar: gtk::ProgressBar,
    split_btn: gtk::Button,

    splitter_pipeline: Option<SplitterPipeline>,
    this_opt: Option<Weak<RefCell<SplitController>>>,
}

impl SplitController {
    pub fn new_rc(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(SplitController {
            base: OutputBaseController::new(builder),

            selected_audio: None,
            toc_visitor: None,
            idx: 0,

            split_list: builder.get_object("split-list-box").unwrap(),
            split_to_flac_row: builder.get_object("flac_split-row").unwrap(),
            flac_warning_lbl: builder.get_object("flac_warning-lbl").unwrap(),
            split_to_wave_row: builder.get_object("wave_split-row").unwrap(),
            wave_warning_lbl: builder.get_object("wave_warning-lbl").unwrap(),
            split_to_opus_row: builder.get_object("opus_split-row").unwrap(),
            opus_warning_lbl: builder.get_object("opus_warning-lbl").unwrap(),
            split_to_vorbis_row: builder.get_object("vorbis_split-row").unwrap(),
            vorbis_warning_lbl: builder.get_object("vorbis_warning-lbl").unwrap(),
            split_to_mp3_row: builder.get_object("mp3_split-row").unwrap(),
            mp3_warning_lbl: builder.get_object("mp3_warning-lbl").unwrap(),
            split_progress_bar: builder.get_object("split-progress").unwrap(),
            split_btn: builder.get_object("split-btn").unwrap(),

            splitter_pipeline: None,
            this_opt: None,
        }));

        {
            let mut this_mut = this.borrow_mut();
            this_mut.this_opt = Some(Rc::downgrade(&this));

            this_mut.split_list.select_row(&this_mut.split_to_flac_row);
            this_mut.cleanup();
        }

        this
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();
        this.have_main_ctrl(main_ctrl);

        this.check_requirements();

        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.split_btn.connect_clicked(move |_| {
            let this_clone = Rc::clone(&this_clone);
            main_ctrl_clone
                .borrow_mut()
                .request_pipeline(Box::new(move |pipeline| {
                    {
                        this_clone.borrow_mut().playback_pipeline = Some(pipeline);
                    }
                    // launch export asynchronoulsy so that main_ctrl is no longer borrowed
                    let this_clone = Rc::clone(&this_clone);
                    gtk::idle_add(move || {
                        this_clone.borrow_mut().split();
                        glib::Continue(false)
                    });
                }));
        });
    }

    pub fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let info = pipeline.info.read().unwrap();
        self.streams_changed(&info);
    }

    pub fn streams_changed(&mut self, info: &MediaInfo) {
        self.selected_audio = info
            .streams
            .selected_audio()
            .map(|selected_audio| selected_audio.to_owned());
        self.split_btn.set_sensitive(self.selected_audio.is_some());
    }

    pub fn cleanup(&mut self) {
        self.split_btn.set_sensitive(false);
        self.split_progress_bar.set_fraction(0f64);
    }

    fn check_requirements(&self) {
        let _ = SplitterPipeline::check_requirements(Format::Flac).map_err(|err| {
            warn!("{}", err);
            self.flac_warning_lbl.set_label(&err);
            self.split_to_flac_row.set_sensitive(false);
        });
        let _ = SplitterPipeline::check_requirements(Format::Wave).map_err(|err| {
            warn!("{}", err);
            self.wave_warning_lbl.set_label(&err);
            self.split_to_wave_row.set_sensitive(false);
        });
        let _ = SplitterPipeline::check_requirements(Format::Opus).map_err(|err| {
            warn!("{}", err);
            self.opus_warning_lbl.set_label(&err);
            self.split_to_opus_row.set_sensitive(false);
        });
        let _ = SplitterPipeline::check_requirements(Format::Vorbis).map_err(|err| {
            warn!("{}", err);
            self.vorbis_warning_lbl.set_label(&err);
            self.split_to_vorbis_row.set_sensitive(false);
        });
        let _ = SplitterPipeline::check_requirements(Format::MP3).map_err(|err| {
            warn!("{}", err);
            self.mp3_warning_lbl.set_label(&err);
            self.split_to_mp3_row.set_sensitive(false);
        });
    }

    fn split(&mut self) {
        // Split button is not sensible when no audio
        // stream is selected (see `streams_changed`)
        debug_assert!(self.selected_audio.is_some());

        let format = self.get_selection();
        self.prepare_process(format, MediaContent::Audio);

        self.toc_visitor = self
            .base
            .playback_pipeline
            .as_ref()
            .unwrap()
            .info
            .read()
            .unwrap()
            .toc
            .as_ref()
            .map(|toc| TocVisitor::new(toc));
        self.idx = 0;

        if let Err(err) = self.build_pipeline(format) {
            self.show_error(err);
        }
    }

    fn build_pipeline(&mut self, format: metadata::Format) -> Result<bool, String> {
        let mut chapter = match self.next_chapter() {
            Some(chapter) => chapter,
            None => {
                if self.toc_visitor.is_none() && self.idx < 2 {
                    // No chapter => build a fake chapter corresponding to the whole file
                    let mut toc_entry =
                        gst::TocEntry::new(gst::TocEntryType::Chapter, &"".to_owned());
                    toc_entry
                        .get_mut()
                        .unwrap()
                        .set_start_stop_times(0, self.duration as i64);

                    let mut tag_list = gst::TagList::new();
                    tag_list.get_mut().unwrap().add::<gst::tags::Title>(
                        &self.media_path.file_stem().unwrap().to_str().unwrap(),
                        gst::TagMergeMode::Replace,
                    );
                    toc_entry.get_mut().unwrap().set_tags(tag_list);

                    toc_entry
                } else {
                    return Ok(false);
                }
            }
        };
        // Unfortunately, we need to make a copy here
        // because the chapter is also owned by the self.toc
        // and the TocVisitor so the chapters entries ref_count is > 1
        let chapter = self.update_tags(&mut chapter);
        let output_path = self.get_split_path(&chapter);
        let (pipeline_tx, ui_rx) = channel();
        self.register_listener(format, LISTENER_PERIOD, ui_rx);
        match SplitterPipeline::try_new(
            &self.media_path,
            &output_path,
            &self.selected_audio.as_ref().unwrap().id,
            format,
            chapter,
            pipeline_tx,
        ) {
            Ok(splitter_pipeline) => {
                self.switch_to_busy();
                self.splitter_pipeline = Some(splitter_pipeline);
                Ok(true)
            }
            Err(error) => {
                self.remove_listener();
                self.switch_to_available();
                self.restore_pipeline();
                let msg = gettext("Failed to prepare for split. {}").replacen("{}", &error, 1);
                error!("{}", msg);
                Err(msg)
            }
        }
    }

    fn next_chapter(&mut self) -> Option<gst::TocEntry> {
        if self.toc_visitor.is_none() {
            self.idx += 1;
            return None;
        }

        let chapter = self.toc_visitor.as_mut().unwrap().next_chapter();

        if chapter.is_some() {
            self.idx += 1;
        }

        chapter
    }

    fn get_split_path(&self, chapter: &gst::TocEntry) -> PathBuf {
        let mut split_name = String::new();

        let info = self
            .playback_pipeline
            .as_ref()
            .unwrap()
            .info
            .read()
            .unwrap();

        // TODO: make format customisable
        if let Some(artist) = info.get_artist() {
            split_name += &format!("{} - ", artist);
        }
        if let Some(album_title) = info.get_title() {
            split_name += &format!("{} - ", album_title);
        }

        let track_title = chapter
            .get_tags()
            .and_then(|tags| {
                tags.get::<gst::tags::Title>()
                    .map(|tag| tag.get().unwrap().to_owned())
            })
            .unwrap_or_else(get_default_chapter_title);

        if self.toc_visitor.is_some() {
            split_name += &format!("{:02}. ", self.idx);
        }

        split_name += &track_title;

        if let Some(ref stream) = self.selected_audio {
            if let Some(ref tags) = stream.tags {
                match tags.get_index::<gst::tags::LanguageName>(0) {
                    Some(ref language) => split_name += &format!(" ({})", language.get().unwrap()),
                    None => {
                        if let Some(ref code) = tags.get_index::<gst::tags::LanguageCode>(0) {
                            split_name += &format!(" ({})", code.get().unwrap());
                        }
                    }
                }
            }
        }

        split_name += &format!(".{}", self.extension);

        self.target_path.with_file_name(split_name)
    }

    fn update_tags(&self, chapter: &mut gst::TocEntry) -> gst::TocEntry {
        let mut tags = gst::TagList::new();
        {
            let tags = tags.get_mut().unwrap();
            let chapter_count = {
                let info = self
                    .playback_pipeline
                    .as_ref()
                    .unwrap()
                    .info
                    .read()
                    .unwrap();

                // Select tags suitable for a track
                for (tag_name, tag_iter) in info.tags.iter_generic() {
                    if TAGS_TO_SKIP
                        .iter()
                        .find(|&&tag_to_skip| tag_to_skip == tag_name)
                        .is_none()
                    {
                        // can add tag
                        for tag_value in tag_iter {
                            if tags
                                .add_generic(tag_name, tag_value, gst::TagMergeMode::Append)
                                .is_err()
                            {
                                warn!(
                                    "{}",
                                    gettext("couldn't add tag {tag_name}").replacen(
                                        "{tag_name}",
                                        tag_name,
                                        1
                                    )
                                );
                            }
                        }
                    }
                }

                info.chapter_count.unwrap_or(1)
            };

            // Add track specific tags
            let title = chapter
                .get_tags()
                .and_then(|tags| {
                    tags.get::<gst::tags::Title>()
                        .map(|tag| tag.get().unwrap().to_owned())
                })
                .unwrap_or_else(get_default_chapter_title);
            tags.add::<gst::tags::Title>(&title.as_str(), gst::TagMergeMode::ReplaceAll);

            let (start, end) = chapter.get_start_stop_times().unwrap();

            tags.add::<gst::tags::TrackNumber>(&(self.idx as u32), gst::TagMergeMode::ReplaceAll);
            tags.add::<gst::tags::TrackCount>(
                &(chapter_count as u32),
                gst::TagMergeMode::ReplaceAll,
            );
            tags.add::<gst::tags::Duration>(
                &gst::ClockTime::from_nseconds((end - start) as u64),
                gst::TagMergeMode::ReplaceAll,
            );
            tags.add::<gst::tags::ApplicationName>(&"media-toc", gst::TagMergeMode::ReplaceAll);
        }

        let chapter = chapter.make_mut();
        chapter.set_tags(tags);
        chapter.to_owned()
    }

    fn get_selection(&self) -> metadata::Format {
        if self.split_to_flac_row.is_selected() {
            Format::Flac
        } else if self.split_to_wave_row.is_selected() {
            Format::Wave
        } else if self.split_to_opus_row.is_selected() {
            Format::Opus
        } else if self.split_to_vorbis_row.is_selected() {
            Format::Vorbis
        } else if self.split_to_mp3_row.is_selected() {
            Format::MP3
        } else {
            unreachable!("ExportController::get_split_selection unknown split type");
        }
    }

    fn switch_to_busy(&self) {
        // TODO: allow cancelling split
        self.base.switch_to_busy();

        self.split_list.set_sensitive(false);
        self.split_btn.set_sensitive(false);
    }

    fn switch_to_available(&self) {
        self.base.switch_to_available();

        self.split_progress_bar.set_fraction(0f64);
        self.split_btn.set_sensitive(true);
        self.split_list.set_sensitive(true);
    }

    fn register_listener(
        &mut self,
        format: metadata::Format,
        period: u32,
        ui_rx: Receiver<PipelineMessage>,
    ) {
        let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(period, move || {
            let mut keep_going = false;

            if let Some(this_rc) = this_weak.upgrade() {
                keep_going = true;
                let mut process_done = false;

                let mut this = this_rc.borrow_mut();

                if this.duration > 0 {
                    let position = match this.splitter_pipeline.as_mut() {
                        Some(splitter_pipeline) => splitter_pipeline.get_position(),
                        None => 0,
                    };
                    this.split_progress_bar
                        .set_fraction(position as f64 / this.duration as f64);
                }

                for message in ui_rx.try_iter() {
                    match message {
                        Eos => {
                            process_done = match this.build_pipeline(format) {
                                Ok(true) => false, // more chapters
                                Ok(false) => {
                                    this.show_info(gettext("Media split succesfully"));
                                    true
                                }
                                Err(err) => {
                                    this.show_error(err);
                                    true
                                }
                            };

                            keep_going = false;
                        }
                        FailedToExport(error) => {
                            this.listener_src = None;
                            keep_going = false;
                            process_done = true;
                            this.show_error(
                                gettext("Failed to split media. {}").replacen("{}", &error, 1),
                            );
                        }
                        _ => (),
                    };

                    if !keep_going {
                        break;
                    }
                }

                if !keep_going && process_done {
                    this.switch_to_available();
                    this.restore_pipeline();
                }
            }

            glib::Continue(keep_going)
        }));
    }
}

impl Deref for SplitController {
    type Target = OutputBaseController;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for SplitController {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
