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
};

use crate::{
    media::{MediaEvent, PlaybackPipeline, SplitterPipeline},
    metadata,
    metadata::{get_default_chapter_title, Format, MediaContent, MediaInfo, Stream, TocVisitor},
};

use super::{
    MainController, OutputBaseController, OutputProcessor, OutputUIController, UIController,
};

const TIMER_PERIOD: u32 = 100; // 100 ms (10 Hz)

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

enum SplitStatus {
    Completed,
    InProgress,
}

macro_rules! update_list_with_format(
    ($self_:expr, $format:expr, $row:ident, $label:ident) => {
        match SplitterPipeline::check_requirements($format) {
            Ok(_) => if $self_.selected_format.is_none() {
                $self_.split_list.select_row(&$self_.$row);
                $self_.selected_format = Some($format);
            },
            Err(err) => {
                warn!("{}", err);
                $self_.$label.set_label(&err);
                $self_.$row.set_sensitive(false);
            }
        }
    };
);

pub struct SplitController {
    base: OutputBaseController,

    selected_format: Option<metadata::Format>,
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

impl UIController for SplitController {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let info = pipeline.info.read().unwrap();
        self.streams_changed(&info);
    }

    fn cleanup(&mut self) {
        self.split_progress_bar.set_fraction(0f64);
        self.split_btn.set_sensitive(false);
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.selected_audio = info
            .streams
            .selected_audio()
            .map(|selected_audio| selected_audio.to_owned());
        if self.selected_format.is_some() {
            self.split_btn.set_sensitive(self.selected_audio.is_some());
        }
    }
}

impl SplitController {
    pub fn new_rc(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(SplitController {
            base: OutputBaseController::new(builder),

            selected_format: None,
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

            this_mut.update_list_with_available_formats();
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
                        this_clone.borrow_mut().start();
                        glib::Continue(false)
                    });
                }));
        });
    }

    fn update_list_with_available_formats(&mut self) {
        update_list_with_format!(self, Format::Flac, split_to_flac_row, flac_warning_lbl);
        update_list_with_format!(self, Format::Wave, split_to_wave_row, wave_warning_lbl);
        update_list_with_format!(self, Format::Opus, split_to_opus_row, opus_warning_lbl);
        update_list_with_format!(
            self,
            Format::Vorbis,
            split_to_vorbis_row,
            vorbis_warning_lbl
        );
        update_list_with_format!(self, Format::MP3, split_to_mp3_row, mp3_warning_lbl);

        let is_usable = self.selected_format.is_some();
        self.split_list.set_sensitive(is_usable);
        self.split_btn.set_sensitive(is_usable);
    }

    fn next(&mut self) -> Result<SplitStatus, String> {
        let mut chapter = match self.toc_visitor.as_mut() {
            Some(toc_visitor) => match toc_visitor.next_chapter() {
                Some(chapter) => {
                    self.idx += 1;
                    chapter
                }
                None => return Ok(SplitStatus::Completed),
            },
            None => {
                // No chapter defined => build a fake chapter corresponding to the whole file
                let mut toc_entry = gst::TocEntry::new(gst::TocEntryType::Chapter, &"".to_owned());
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
                self.idx += 1;

                toc_entry
            }
        };

        // Unfortunately, we need to make a copy here
        // because the chapter is also owned by the self.toc
        // and the TocVisitor so the chapters entries ref_count is > 1
        let chapter = self.update_tags(&mut chapter);
        let output_path = self.get_split_path(&chapter);

        let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
        self.register_media_event_handler(receiver);

        let splitter_pipeline = SplitterPipeline::try_new(
            &self.media_path,
            &output_path,
            &self.selected_audio.as_ref().unwrap().id,
            self.selected_format
                .expect("No selected format in `SplitterController`"),
            chapter,
            sender,
        )
        .map_err(|err| gettext("Failed to prepare for split. {}").replacen("{}", &err, 1))?;

        self.splitter_pipeline = Some(splitter_pipeline);
        Ok(SplitStatus::InProgress)
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
            unreachable!("`SplitController`: unknown split type");
        }
    }

    fn register_timer(&mut self, period: u32) {
        if self.timer_src.is_none() {
            let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

            self.timer_src = Some(glib::timeout_add_local(period, move || {
                let this_rc = this_weak
                    .upgrade()
                    .expect("Lost `SplitController` in timer");
                this_rc.borrow_mut().update_progress();

                glib::Continue(true)
            }));
        }
    }

    fn register_media_event_handler(&mut self, receiver: glib::Receiver<MediaEvent>) {
        let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

        receiver.attach(None, move |event| {
            let this_rc = this_weak
                .upgrade()
                .expect("Lost `SplitController` in `MediaEvent` handler");
            let mut this = this_rc.borrow_mut();
            this.handle_media_event(event)
        });
    }
}

impl OutputProcessor for SplitController {
    fn start(&mut self) {
        // Split button is not sensible when no audio
        // stream is selected (see `streams_changed`)
        debug_assert!(self.selected_audio.is_some());

        // FIXME: update `selected_format` when list selection is changed
        let format = self.get_selection();
        self.prepare_process(format, MediaContent::Audio);
        self.selected_format = Some(format);

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

        match self.next() {
            Ok(SplitStatus::InProgress) => {
                self.switch_to_busy();
                self.register_timer(TIMER_PERIOD);
            }
            Ok(SplitStatus::Completed) => {
                unreachable!("`SplitController`: split completed immediately")
            }
            Err(err) => {
                error!("{}", err);
                self.switch_to_available();
                self.restore_pipeline();
                self.show_error(err);
            }
        }
    }

    fn handle_media_event(&mut self, event: MediaEvent) -> glib::Continue {
        let mut keep_going = true;
        let mut process_done = false;
        match event {
            MediaEvent::Eos => {
                match self.next() {
                    Ok(SplitStatus::InProgress) => (),
                    Ok(SplitStatus::Completed) => {
                        self.show_info(gettext("Media split succesfully"));
                        process_done = true;
                    }
                    Err(err) => {
                        self.show_error(err);
                        process_done = true;
                    }
                }

                keep_going = false;
            }
            MediaEvent::FailedToExport(err) => {
                keep_going = false;
                process_done = true;
                self.show_error(gettext("Failed to split media. {}").replacen("{}", &err, 1));
            }
            _ => (),
        }

        if !keep_going && process_done {
            self.switch_to_available();
            self.restore_pipeline();
        }

        glib::Continue(keep_going)
    }
}

impl OutputUIController for SplitController {
    fn switch_to_busy(&self) {
        // TODO: allow cancelling split
        self.base.switch_to_busy();

        self.split_list.set_sensitive(false);
        self.split_btn.set_sensitive(false);
    }

    fn switch_to_available(&mut self) {
        self.base.switch_to_available();

        let is_usable = self.selected_format.is_some();
        self.split_list.set_sensitive(is_usable);
        self.split_btn.set_sensitive(is_usable);
    }

    fn update_progress(&mut self) {
        if self.duration > 0 {
            let position = match self.splitter_pipeline.as_mut() {
                Some(splitter_pipeline) => splitter_pipeline.get_position(),
                None => 0,
            };
            self.split_progress_bar
                .set_fraction(position as f64 / self.duration as f64);
        }
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
