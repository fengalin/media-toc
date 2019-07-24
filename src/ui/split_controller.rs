use gettextrs::gettext;
use glib;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;
use log::warn;

use std::{
    borrow::ToOwned,
    path::Path,
    rc::Rc,
    string::ToString,
    sync::{Arc, RwLock},
};

use media::{MediaEvent, SplitterPipeline};
use metadata::{get_default_chapter_title, Duration, Format, MediaInfo, Stream, TocVisitor};

use super::{
    MediaProcessor, OutputBaseController, OutputControllerImpl, OutputMediaFileInfo,
    PlaybackPipeline, ProcessingState, ProcessingType, UIController, UIEventSender, UIFocusContext,
};

pub type SplitController = OutputBaseController<SplitControllerImpl>;

impl SplitController {
    pub fn new(builder: &gtk::Builder, ui_event: UIEventSender) -> Self {
        OutputBaseController::<SplitControllerImpl>::new_base(
            SplitControllerImpl::new(builder),
            builder,
            ui_event,
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

pub struct SplitControllerImpl {
    is_usable: bool,

    src_info: Option<Arc<RwLock<MediaInfo>>>,
    selected_audio: Option<Stream>,

    split_file_info: Option<OutputMediaFileInfo>,
    media_event_sender: Option<glib::Sender<MediaEvent>>,
    splitter_pipeline: Option<SplitterPipeline>,
    toc_visitor: Option<TocVisitor>,
    idx: usize,
    current_chapter: Option<gst::TocEntry>,
    current_path: Option<Rc<Path>>,

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
    const FOCUS_CONTEXT: UIFocusContext = UIFocusContext::SplitPage;
    const BTN_NAME: &'static str = "split-btn";
    const LIST_NAME: &'static str = "split-list-box";
    const PROGRESS_BAR_NAME: &'static str = "split-progress";
}

impl UIController for SplitControllerImpl {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let info_arc = Arc::clone(&pipeline.info);
        self.src_info = Some(info_arc);
    }

    fn cleanup(&mut self) {
        self.src_info = None;
        self.selected_audio = None;

        self.reset();
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.selected_audio = info.streams.selected_audio().map(ToOwned::to_owned);
        if self.is_usable {
            self.split_btn.set_sensitive(self.selected_audio.is_some());
        }
    }
}

impl SplitControllerImpl {
    pub fn new(builder: &gtk::Builder) -> Self {
        let mut ctrl = SplitControllerImpl {
            is_usable: false,

            src_info: None,
            selected_audio: None,

            split_file_info: None,
            media_event_sender: None,
            splitter_pipeline: None,
            toc_visitor: None,
            idx: 0,
            current_chapter: None,
            current_path: None,

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

    fn reset(&mut self) {
        self.split_file_info = None;
        self.media_event_sender = None;
        self.splitter_pipeline = None;
        self.toc_visitor = None;
        self.idx = 0;
        self.current_chapter = None;
        self.current_path = None;
    }

    fn get_split_path(&self, chapter: &gst::TocEntry) -> Rc<Path> {
        let mut split_name = String::new();

        let src_info = self.src_info.as_ref().unwrap().read().unwrap();

        // TODO: make format customisable
        let artist = src_info
            .get_media_artist_sortname()
            .or_else(|| src_info.get_media_artist());
        if let Some(artist) = artist {
            split_name += &format!("{} - ", artist);
        }

        let album_title = src_info
            .get_media_title_sortname()
            .or_else(|| src_info.get_media_title());
        if let Some(album_title) = album_title {
            split_name += &format!("{} - ", album_title);
        }

        if self.toc_visitor.is_some() {
            split_name += &format!("{:02}", self.idx);
        }

        let track_title = chapter
            .get_tags()
            .and_then(|tags| {
                tags.get::<gst::tags::Title>()
                    .and_then(|tag| tag.get().map(ToString::to_string))
            })
            .unwrap_or_else(get_default_chapter_title);
        split_name += &format!(". {}", track_title);

        let lang = self.selected_audio.as_ref().and_then(|stream| {
            stream
                .tags
                .get_index::<gst::tags::LanguageName>(0)
                .or_else(|| stream.tags.get_index::<gst::tags::LanguageCode>(0))
                .and_then(|value| value.get().map(ToString::to_string))
        });
        if let Some(lang) = lang {
            split_name += &format!(" ({})", lang);
        }

        let split_file_info = self.split_file_info.as_ref().expect(concat!(
            "SplitControllerImpl: split_file_info not defined in `get_split_path()`, ",
            "did you call `init()`?"
        ));

        split_name += &format!(".{}", split_file_info.extension);

        split_file_info.path.with_file_name(split_name).into()
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

        self.split_file_info = Some({
            let src_info = self.src_info.as_ref().unwrap().read().unwrap();
            OutputMediaFileInfo::new(format, &src_info)
        });

        let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
        self.media_event_sender = Some(sender);

        ProcessingType::Async(receiver)
    }

    fn next(&mut self) -> Result<ProcessingState, String> {
        let chapter = match self.toc_visitor.as_mut() {
            Some(toc_visitor) => match toc_visitor.next_chapter() {
                Some(chapter) => {
                    self.idx += 1;
                    chapter
                }
                None => {
                    self.split_file_info = None;
                    return Ok(ProcessingState::AllComplete(gettext(
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
                    .set_start_stop_times(0, src_info.duration.as_i64());

                let mut tag_list = gst::TagList::new();
                tag_list.get_mut().unwrap().add::<gst::tags::Title>(
                    &src_info.path.file_stem().unwrap().to_str().unwrap(),
                    gst::TagMergeMode::Replace,
                );
                toc_entry.get_mut().unwrap().set_tags(tag_list);

                toc_entry
            }
        };

        self.current_chapter = Some(
            self.src_info
                .as_ref()
                .unwrap()
                .read()
                .unwrap()
                .get_chapter_with_track_tags(&chapter, self.idx),
        );

        let split_path = self.get_split_path(&chapter);
        self.current_path = Some(Rc::clone(&split_path));

        Ok(ProcessingState::WouldOutputTo(split_path))
    }

    fn process(&mut self, output_path: &Path) -> Result<ProcessingState, String> {
        let res = {
            let src_info = self.src_info.as_ref().unwrap().read().unwrap();
            let split_file_info = self.split_file_info.as_ref().expect(
                "SplitControllerImpl: split_file_info not defined in `next()`, did you call `init()`?"
            );
            SplitterPipeline::try_new(
                &src_info.path,
                output_path,
                &self.selected_audio.as_ref().unwrap().id,
                split_file_info.format,
                self.current_chapter.take().expect(concat!(
                    "SplitControllerImpl: no current_chapter ",
                    "in `process_current_chapter()`",
                )),
                self.media_event_sender
                    .as_ref()
                    .expect(concat!(
                        "SplitControllerImpl: no media_event_sender in `process_current_chapter()` ",
                        "did you call `init()`?",
                    ))
                    .clone(),
            )
        };

        self.splitter_pipeline = Some(res.map_err(|err| {
            self.reset();
            gettext("Failed to prepare for split. {}").replacen("{}", &err, 1)
        })?);

        Ok(ProcessingState::PendingAsyncMediaEvent)
    }

    fn cancel(&mut self) {
        self.splitter_pipeline
            .as_mut()
            .expect("SplitController::cancel no splitter_pipeline")
            .cancel();

        if let Some(current_path) = self.current_path.take() {
            if std::fs::remove_file(&current_path).is_err() {
                if let Some(printable_path) = current_path.to_str() {
                    warn!("Failed to remove canceled split file {}", printable_path);
                }
            }
        }

        self.reset();
    }

    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingState, String> {
        match event {
            MediaEvent::Eos => {
                self.current_chapter = None;
                self.current_path = None;
                Ok(ProcessingState::DoneWithCurrent)
            }
            MediaEvent::FailedToExport(err) => {
                self.reset();
                Err(gettext("Failed to split media. {}").replacen("{}", &err, 1))
            }
            other => unimplemented!("SplitController: can't handle media event {:?}", other),
        }
    }

    fn report_progress(&self) -> Option<f64> {
        let duration = self.src_info.as_ref().unwrap().read().unwrap().duration;
        if duration > Duration::default() {
            self.splitter_pipeline
                .as_ref()
                .map(SplitterPipeline::get_current_ts)?
                .map(|ts| ts.as_f64() / duration.as_f64())
        } else {
            Some(0f64)
        }
    }
}
