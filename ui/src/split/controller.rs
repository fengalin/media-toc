use futures::channel::mpsc as async_mpsc;

use gettextrs::gettext;
use gtk::prelude::*;
use log::warn;

use std::{
    path::Path,
    rc::Rc,
    sync::{Arc, RwLock},
};

use media::{MediaEvent, PlaybackPipeline, SplitterPipeline};
use metadata::{default_chapter_title, Duration, Format, MediaInfo, Stream, TocVisitor};

use crate::{
    generic_output::{self, prelude::*},
    prelude::*,
    split,
};

pub type Controller = generic_output::Controller<ControllerImpl>;

impl Controller {
    pub fn new(builder: &gtk::Builder) -> Self {
        generic_output::Controller::<ControllerImpl>::new_generic(
            ControllerImpl::new(builder),
            builder,
        )
    }
}

macro_rules! update_list_with_format(
    ($self_:expr, $format:expr, $row:ident, $label:ident) => {
        match SplitterPipeline::check_requirements($format) {
            Ok(_) => if !$self_.is_usable {
                $self_.split_list.select_row(Some(&$self_.$row));
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

pub struct ControllerImpl {
    is_usable: bool,

    src_info: Option<Arc<RwLock<MediaInfo>>>,
    selected_audio: Option<Stream>,

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

impl OutputControllerImpl for ControllerImpl {
    type MediaProcessorImplType = Processor;
    type OutputEvent = split::Event;

    const FOCUS_CONTEXT: UIFocusContext = UIFocusContext::SplitPage;
    const BTN_NAME: &'static str = "split-btn";
    const LIST_NAME: &'static str = "split-list-box";
    const PROGRESS_BAR_NAME: &'static str = "split-progress";

    fn new_processor(&self) -> Processor {
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

        // Split button is not sensitive when no audio
        // stream is selected (see `streams_changed`)
        debug_assert!(self.selected_audio.is_some());

        Processor {
            src_info: Arc::clone(self.src_info.as_ref().unwrap()),
            selected_audio: self.selected_audio.clone(),
            split_file_info: Some({
                let src_info = self.src_info.as_ref().unwrap().read().unwrap();
                OutputMediaFileInfo::new(format, &src_info)
            }),
            toc_visitor: self
                .src_info
                .as_ref()
                .unwrap()
                .read()
                .unwrap()
                .toc
                .as_ref()
                .map(|toc| TocVisitor::new(toc)),
            splitter_pipeline: None,
            idx: 0,
            current_chapter: None,
            current_path: None,
            last_progress: 0f64,
        }
    }
}

impl UIController for ControllerImpl {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let info_arc = Arc::clone(&pipeline.info);
        self.src_info = Some(info_arc);
    }

    fn cleanup(&mut self) {
        self.src_info = None;
        self.selected_audio = None;
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.selected_audio = info.streams.selected_audio().map(Stream::to_owned);
        if self.is_usable {
            self.split_btn.set_sensitive(self.selected_audio.is_some());
        }
    }
}

impl ControllerImpl {
    pub fn new(builder: &gtk::Builder) -> Self {
        let mut ctrl = ControllerImpl {
            is_usable: false,

            src_info: None,
            selected_audio: None,

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
        };

        update_list_with_format!(ctrl, Format::Flac, split_to_flac_row, flac_warning_lbl);
        update_list_with_format!(ctrl, Format::Wave, split_to_wave_row, wave_warning_lbl);
        update_list_with_format!(ctrl, Format::Opus, split_to_opus_row, opus_warning_lbl);
        update_list_with_format!(
            ctrl,
            Format::Vorbis,
            split_to_vorbis_row,
            vorbis_warning_lbl
        );
        update_list_with_format!(ctrl, Format::MP3, split_to_mp3_row, mp3_warning_lbl);

        ctrl.split_list.set_sensitive(ctrl.is_usable);
        ctrl.split_btn.set_sensitive(ctrl.is_usable);

        ctrl
    }
}

pub struct Processor {
    src_info: Arc<RwLock<MediaInfo>>,
    selected_audio: Option<Stream>,

    split_file_info: Option<OutputMediaFileInfo>,
    idx: usize,
    toc_visitor: Option<TocVisitor>,
    splitter_pipeline: Option<SplitterPipeline>,
    last_progress: f64,
    current_chapter: Option<gst::TocEntry>,
    current_path: Option<Rc<Path>>,
}

impl Processor {
    fn split_path(&self, chapter: &gst::TocEntry) -> Rc<Path> {
        let mut split_name = String::new();

        let src_info = self.src_info.read().unwrap();

        // TODO: make format customisable
        let artist = src_info
            .media_artist_sortname()
            .or_else(|| src_info.media_artist());
        if let Some(artist) = artist {
            split_name += &format!("{} - ", artist);
        }

        let album_title = src_info
            .media_title_sortname()
            .or_else(|| src_info.media_title());
        if let Some(album_title) = album_title {
            split_name += &format!("{} - ", album_title);
        }

        if self.toc_visitor.is_some() {
            split_name += &format!("{:02}. ", self.idx);
        }

        let track_title = chapter
            .get_tags()
            .and_then(|tags| {
                tags.get::<gst::tags::Title>()
                    .and_then(|tag| tag.get().map(str::to_string))
            })
            .unwrap_or_else(default_chapter_title);

        split_name += &track_title;

        let lang = self.selected_audio.as_ref().and_then(|stream| {
            stream
                .tags
                .get_index::<gst::tags::LanguageName>(0)
                .or_else(|| stream.tags.get_index::<gst::tags::LanguageCode>(0))
                .and_then(|value| value.get().map(str::to_string))
        });
        if let Some(lang) = lang {
            split_name += &format!(" ({})", lang);
        }

        let split_file_info = self.split_file_info.as_ref().unwrap();

        split_name += &format!(".{}", split_file_info.extension);

        split_file_info.path.with_file_name(split_name).into()
    }
}

impl Iterator for Processor {
    type Item = Rc<Path>;

    fn next(&mut self) -> Option<Rc<Path>> {
        let chapter = self
            .toc_visitor
            .as_mut()
            .and_then(TocVisitor::next_chapter)
            .or_else(|| {
                if self.idx == 0 {
                    // No chapter defined => build a fake chapter corresponding to the whole file

                    let src_info = self.src_info.read().unwrap();
                    let mut toc_entry =
                        gst::TocEntry::new(gst::TocEntryType::Chapter, &"".to_owned());
                    toc_entry
                        .get_mut()
                        .unwrap()
                        .set_start_stop_times(0, src_info.duration.as_i64());

                    let mut tag_list = gst::TagList::new();
                    tag_list.get_mut().unwrap().add::<gst::tags::Title>(
                        &src_info.path.file_stem().unwrap().to_str().unwrap(),
                        gst::TagMergeMode::Replace,
                    );
                    toc_entry.get_mut().unwrap().set_tags(tag_list);

                    Some(toc_entry)
                } else {
                    None
                }
            });

        if chapter.is_none() {
            self.split_file_info = None;
            return None;
        }

        let chapter = chapter.as_ref().unwrap();

        self.idx += 1;

        self.current_chapter = Some(
            self.src_info
                .read()
                .unwrap()
                .chapter_with_track_tags(chapter, self.idx),
        );

        let split_path = self.split_path(chapter);
        self.current_path = Some(Rc::clone(&split_path));

        Some(split_path)
    }
}

impl MediaProcessorImpl for Processor {
    fn process(&mut self, output_path: &Path) -> Result<ProcessingType, MediaProcessorError> {
        let (res, receiver) = {
            let src_info = self.src_info.read().unwrap();
            let split_file_info = self.split_file_info.as_ref().unwrap();

            let stream_id = if src_info.streams.collection(gst::StreamType::AUDIO).len() > 1 {
                Some(self.selected_audio.as_ref().unwrap().id.to_string())
            } else {
                // Some single stream decoders advertise a random id at each invocation
                // so don't be explicit when only one audio stream is available
                None
            };

            let (sender, receiver) = async_mpsc::channel(MEDIA_EVENT_CHANNEL_CAPACITY);

            let res = SplitterPipeline::try_new(
                &src_info.path,
                output_path,
                stream_id,
                split_file_info.format,
                self.current_chapter.take().expect("no current_chapter"),
                sender,
            );

            (res, receiver)
        };

        self.splitter_pipeline =
            Some(res.map_err(|err| {
                gettext("Failed to prepare for split. {}").replacen("{}", &err, 1)
            })?);

        Ok(ProcessingType::Async(receiver))
    }

    fn cancel(&mut self) {
        if let Some(pipeline) = self.splitter_pipeline.as_mut() {
            pipeline.cancel();

            if let Some(current_path) = self.current_path.take() {
                if std::fs::remove_file(&current_path).is_err() {
                    if let Some(printable_path) = current_path.to_str() {
                        warn!("Failed to remove canceled split file {}", printable_path);
                    }
                }
            }
        }
    }

    fn handle_media_event(
        &mut self,
        event: MediaEvent,
    ) -> Result<MediaEventHandling, MediaProcessorError> {
        match event {
            MediaEvent::Eos => {
                self.current_chapter = None;
                self.current_path = None;
                self.splitter_pipeline = None;
                Ok(MediaEventHandling::Done)
            }
            MediaEvent::FailedToExport(err) => Err(gettext("Failed to split media. {}")
                .replacen("{}", &err, 1)
                .into()),
            other => unimplemented!("split::Controller: can't handle media event {:?}", other),
        }
    }

    fn report_progress(&mut self) -> f64 {
        let duration = self.src_info.read().unwrap().duration;
        if duration > Duration::default() {
            // With some formats, we can't retrieve a proper ts between 2 files
            // so, just report known last progress in this case
            if let Some(ts) = self
                .splitter_pipeline
                .as_ref()
                .and_then(SplitterPipeline::current_ts)
            {
                self.last_progress = ts.as_f64() / duration.as_f64()
            }

            self.last_progress
        } else {
            0f64
        }
    }

    fn completion_msg() -> String {
        gettext("Media split succesfully")
    }
}
