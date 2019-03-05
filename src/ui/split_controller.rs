use gettextrs::gettext;
use glib;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;
use log::warn;

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, RwLock},
};

use crate::{
    media::{MediaEvent, PlaybackPipeline, SplitterPipeline},
    metadata::{get_default_chapter_title, Format, MediaInfo, Stream, TocVisitor},
};

use super::{
    MainController, MediaProcessor, OutputBaseController, OutputControllerImpl,
    OutputMediaFileInfo, ProcessingStatus, ProcessingType, UIController,
};

pub type SplitController = OutputBaseController<SplitControllerImpl>;

impl SplitController {
    pub fn new_rc(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        OutputBaseController::<SplitControllerImpl>::new_base_rc(
            SplitControllerImpl::new(builder),
            builder,
        )
    }
}

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

macro_rules! update_list_with_format(
    ($self_:expr, $format:expr, $row:ident, $label:ident) => {
        match SplitterPipeline::check_requirements($format) {
            Ok(_) => if !$self_.is_usable {
                $self_.split_list.select_row(&$self_.$row);
                $self_.is_usable = true;
            },
            Err(err) => {
                warn!("{}", err);
                $self_.$label.set_label(&err);
                $self_.$row.set_sensitive(false);
            }
        }
    };
);

pub struct SplitControllerImpl {
    is_usable: bool,

    src_info: Option<Arc<RwLock<MediaInfo>>>,
    selected_audio: Option<Stream>,

    split_file_info: Option<OutputMediaFileInfo>,
    sender: Option<glib::Sender<MediaEvent>>,
    splitter_pipeline: Option<SplitterPipeline>,
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

    split_btn: gtk::Button,
}

impl OutputControllerImpl for SplitControllerImpl {
    const BTN_NAME: &'static str = "split-btn";
    const LIST_NAME: &'static str = "split-list-box";
    const PROGRESS_BAR_NAME: &'static str = "split-progress";
}

impl UIController for SplitControllerImpl {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let info_arc = Arc::clone(&pipeline.info);

        {
            let info = info_arc.read().unwrap();
            self.streams_changed(&info);
        }

        self.src_info = Some(info_arc);
    }

    fn cleanup(&mut self) {
        self.src_info = None;
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.selected_audio = info
            .streams
            .selected_audio()
            .map(|selected_audio| selected_audio.to_owned());
        if self.is_usable {
            self.split_btn.set_sensitive(self.selected_audio.is_some());
        }
    }

    fn setup(&mut self, _gtk_app: &gtk::Application, _main_ctrl: &Rc<RefCell<MainController>>) {
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

        self.split_list.set_sensitive(self.is_usable);
        self.split_btn.set_sensitive(self.is_usable);
    }
}

impl SplitControllerImpl {
    pub fn new(builder: &gtk::Builder) -> Self {
        SplitControllerImpl {
            is_usable: false,

            src_info: None,
            selected_audio: None,

            split_file_info: None,
            sender: None,
            splitter_pipeline: None,
            toc_visitor: None,
            idx: 0,

            split_list: builder.get_object(Self::LIST_NAME).unwrap(),
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

            split_btn: builder.get_object(Self::BTN_NAME).unwrap(),
        }
    }

    fn next(&mut self) -> Result<ProcessingStatus, String> {
        let mut chapter = match self.toc_visitor.as_mut() {
            Some(toc_visitor) => match toc_visitor.next_chapter() {
                Some(chapter) => {
                    self.idx += 1;
                    chapter
                }
                None => {
                    self.split_file_info = None;
                    return Ok(ProcessingStatus::Completed(gettext(
                        "Media split succesfully",
                    )));
                }
            },
            None => {
                // No chapter defined => build a fake chapter corresponding to the whole file
                self.idx += 1;

                let src_info = self.src_info.as_ref().unwrap().read().unwrap();
                let mut toc_entry = gst::TocEntry::new(gst::TocEntryType::Chapter, &"".to_owned());
                toc_entry
                    .get_mut()
                    .unwrap()
                    .set_start_stop_times(0, src_info.duration as i64);

                let mut tag_list = gst::TagList::new();
                tag_list.get_mut().unwrap().add::<gst::tags::Title>(
                    &src_info.path.file_stem().unwrap().to_str().unwrap(),
                    gst::TagMergeMode::Replace,
                );
                toc_entry.get_mut().unwrap().set_tags(tag_list);

                toc_entry
            }
        };

        // Unfortunately, we need to make a copy here
        // because the chapter is also owned by the self.toc
        // and the TocVisitor so the chapters entries ref_count is > 1
        let chapter = self.update_tags(&mut chapter);
        let output_path = self.get_split_path(&chapter);

        let res = {
            let src_info = self.src_info.as_ref().unwrap().read().unwrap();
            let split_file_info = self.split_file_info.as_ref().expect(
                "SplitControllerImpl: split_file_info not defined in `next()`, did you call `init()`?"
            );
            SplitterPipeline::try_new(
                &src_info.path,
                &output_path,
                &self.selected_audio.as_ref().unwrap().id,
                split_file_info.format,
                chapter,
                self.sender
                    .as_ref()
                    .expect("SplitControllerImpl: no sender in `next()` did you call `init()`?")
                    .clone(),
            )
        };

        self.splitter_pipeline = Some(res.map_err(|err| {
            self.split_file_info = None;
            gettext("Failed to prepare for split. {}").replacen("{}", &err, 1)
        })?);

        Ok(ProcessingStatus::InProgress)
    }

    fn get_split_path(&self, chapter: &gst::TocEntry) -> PathBuf {
        let mut split_name = String::new();

        let src_info = self.src_info.as_ref().unwrap().read().unwrap();

        // TODO: make format customisable
        if let Some(artist) = src_info.get_artist() {
            split_name += &format!("{} - ", artist);
        }
        if let Some(album_title) = src_info.get_title() {
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

        let split_file_info = self.split_file_info.as_ref().expect(concat!(
            "SplitControllerImpl: split_file_info not defined in `get_split_path()`, ",
            "did you call `init()`?"
        ));

        split_name += &format!(".{}", split_file_info.extension);

        split_file_info.path.with_file_name(split_name)
    }

    fn update_tags(&self, chapter: &mut gst::TocEntry) -> gst::TocEntry {
        let mut tags = gst::TagList::new();
        {
            let tags = tags.get_mut().unwrap();
            let chapter_count = {
                let src_info = self.src_info.as_ref().unwrap().read().unwrap();

                // Select tags suitable for a track
                for (tag_name, tag_iter) in src_info.tags.iter_generic() {
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

                src_info.chapter_count.unwrap_or(1)
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
}

impl MediaProcessor for SplitControllerImpl {
    fn init(&mut self) -> ProcessingType {
        let format = if self.split_to_flac_row.is_selected() {
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
        };

        self.split_file_info = Some({
            let src_info = self.src_info.as_ref().unwrap().read().unwrap();
            OutputMediaFileInfo::new(format, &src_info)
        });

        let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
        self.sender = Some(sender);

        ProcessingType::Async(receiver)
    }

    fn start(&mut self) -> Result<ProcessingStatus, String> {
        // Split button is not sensible when no audio
        // stream is selected (see `streams_changed`)
        debug_assert!(self.selected_audio.is_some());

        self.toc_visitor = self
            .src_info
            .as_ref()
            .unwrap()
            .read()
            .unwrap()
            .toc
            .as_ref()
            .map(|toc| TocVisitor::new(toc));
        self.idx = 0;

        match self.next()? {
            ProcessingStatus::InProgress => Ok(ProcessingStatus::InProgress),
            ProcessingStatus::Completed(_) => {
                unreachable!("`SplitController`: split completed immediately")
            }
        }
    }

    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingStatus, String> {
        match event {
            MediaEvent::Eos => self.next(),
            MediaEvent::FailedToExport(err) => {
                Err(gettext("Failed to split media. {}").replacen("{}", &err, 1))
            }
            _ => Ok(ProcessingStatus::InProgress),
        }
    }

    fn report_progress(&mut self) -> f64 {
        let duration = self.src_info.as_ref().unwrap().read().unwrap().duration;
        if duration > 0 {
            let position = match self.splitter_pipeline.as_mut() {
                Some(splitter_pipeline) => splitter_pipeline.get_position(),
                None => 0,
            };
            position as f64 / duration as f64
        } else {
            0f64
        }
    }
}
