use gettextrs::gettext;
use glib;
use gstreamer as gst;

use gtk;
use gtk::prelude::*;

use std::cell::RefCell;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::mpsc::{channel, Receiver};

use media::{ContextMessage, PlaybackContext, SplitterContext, TocSetterContext};
use media::ContextMessage::*;

use metadata;
use metadata::{get_default_chapter_title, Exporter, Format, MatroskaTocFormat, TocVisitor};

use super::MainController;

const LISTENER_PERIOD: u32 = 250; // 250 ms (4 Hz)

macro_rules! add_tag_from(
    ($tags:expr, $original_tags:expr, $TagType:ty) => {
        if let Some(tag) = $original_tags.get_index::<$TagType>(0) {
            $tags.add::<$TagType>(tag.get().as_ref().unwrap(), gst::TagMergeMode::Replace);
        }
    };
);

#[derive(Clone, PartialEq)]
enum ExportType {
    ExternalToc,
    SingleFileWithToc,
}

pub struct ExportController {
    perspective_selector: gtk::MenuButton,
    open_btn: gtk::Button,
    chapter_grid: gtk::Grid,

    export_list: gtk::ListBox,
    mkvmerge_txt_row: gtk::ListBoxRow,
    mkvmerge_txt_warning_lbl: gtk::Label,
    cue_row: gtk::ListBoxRow,
    mkv_row: gtk::ListBoxRow,
    export_progress_bar: gtk::ProgressBar,
    export_btn: gtk::Button,

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

    pub playback_ctx: Option<PlaybackContext>,
    toc_setter_ctx: Option<TocSetterContext>,
    splitter_ctx: Option<SplitterContext>,
    media_path: PathBuf,
    target_path: PathBuf,
    extension: String,
    idx: usize,
    toc_visitor: Option<TocVisitor>,
    duration: u64,

    this_opt: Option<Rc<RefCell<ExportController>>>,
    listener_src: Option<glib::SourceId>,

    main_ctrl: Option<Weak<RefCell<MainController>>>,
}

impl ExportController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(ExportController {
            perspective_selector: builder.get_object("perspective-menu-btn").unwrap(),
            open_btn: builder.get_object("open-btn").unwrap(),
            chapter_grid: builder.get_object("info-chapter_list-grid").unwrap(),

            export_list: builder.get_object("export-list-box").unwrap(),
            mkvmerge_txt_row: builder.get_object("mkvmerge_text_export-row").unwrap(),
            mkvmerge_txt_warning_lbl: builder.get_object("mkvmerge_text_warning-lbl").unwrap(),
            cue_row: builder.get_object("cue_sheet_export-row").unwrap(),
            mkv_row: builder.get_object("matroska_export-row").unwrap(),
            export_progress_bar: builder.get_object("export-progress").unwrap(),
            export_btn: builder.get_object("export-btn").unwrap(),

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

            playback_ctx: None,
            toc_setter_ctx: None,
            splitter_ctx: None,
            media_path: PathBuf::new(),
            target_path: PathBuf::new(),
            extension: String::new(),
            idx: 0,
            toc_visitor: None,
            duration: 0,

            this_opt: None,
            listener_src: None,

            main_ctrl: None,
        }));

        {
            let mut this_mut = this.borrow_mut();
            let this_rc = Rc::clone(&this);
            this_mut.this_opt = Some(this_rc);

            this_mut.export_list.select_row(&this_mut.mkvmerge_txt_row);
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

        this.check_requirements();

        this.main_ctrl = Some(Rc::downgrade(main_ctrl));

        // Export
        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.export_btn.connect_clicked(move |_| {
            let this_clone = Rc::clone(&this_clone);
            main_ctrl_clone
                .borrow_mut()
                .request_context(Box::new(move |context| {
                    {
                        this_clone.borrow_mut().playback_ctx = Some(context);
                    }
                    // launch export asynchronoulsy so that main_ctrl is no longer borrowed
                    let this_clone = Rc::clone(&this_clone);
                    gtk::idle_add(move || {
                        this_clone.borrow_mut().export();
                        glib::Continue(false)
                    });
                }));
        });

        // Split
        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.split_btn.connect_clicked(move |_| {
            let this_clone = Rc::clone(&this_clone);
            main_ctrl_clone
                .borrow_mut()
                .request_context(Box::new(move |context| {
                    {
                        this_clone.borrow_mut().playback_ctx = Some(context);
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

    pub fn new_media(&mut self, _context: &PlaybackContext) {
        self.export_btn.set_sensitive(true);
        self.split_btn.set_sensitive(true);
    }

    pub fn cleanup(&mut self) {
        self.export_btn.set_sensitive(false);
        self.split_btn.set_sensitive(false);
        self.export_progress_bar.set_fraction(0f64);
        self.split_progress_bar.set_fraction(0f64);
    }

    fn check_requirements(&self) {
        if let Err(err) = TocSetterContext::check_requirements() {
            warn!("{}", err);
            self.mkvmerge_txt_warning_lbl.set_label(&err);
            self.mkv_row.set_sensitive(false)
        }

        if let Err(err) = SplitterContext::check_requirements(Format::Flac) {
            warn!("{}", err);
            self.flac_warning_lbl.set_label(&err);
            self.split_to_flac_row.set_sensitive(false)
        }

        if let Err(err) = SplitterContext::check_requirements(Format::Wave) {
            warn!("{}", err);
            self.wave_warning_lbl.set_label(&err);
            self.split_to_wave_row.set_sensitive(false)
        }
        if let Err(err) = SplitterContext::check_requirements(Format::Opus) {
            warn!("{}", err);
            self.opus_warning_lbl.set_label(&err);
            self.split_to_opus_row.set_sensitive(false)
        }
        if let Err(err) = SplitterContext::check_requirements(Format::Vorbis) {
            warn!("{}", err);
            self.vorbis_warning_lbl.set_label(&err);
            self.split_to_vorbis_row.set_sensitive(false)
        }
        if let Err(err) = SplitterContext::check_requirements(Format::MP3) {
            warn!("{}", err);
            self.mp3_warning_lbl.set_label(&err);
            self.split_to_mp3_row.set_sensitive(false)
        }
    }

    fn prepare_process(&mut self, format: &metadata::Format) {
        self.switch_to_busy();

        self.toc_visitor = self.playback_ctx
            .as_ref()
            .unwrap()
            .info
            .lock()
            .unwrap()
            .toc
            .as_ref()
            .map(|toc| TocVisitor::new(toc));

        let is_audio_only = {
            self.playback_ctx
                .as_ref()
                .unwrap()
                .info
                .lock()
                .unwrap()
                .streams
                .video_selected
                .is_none()
        };
        self.extension = metadata::Factory::get_extension(format, is_audio_only).to_owned();

        self.media_path = self.playback_ctx.as_ref().unwrap().path.clone();
        self.target_path = self.media_path.with_extension(&self.extension);

        self.idx = 0;

        if self.listener_src.is_some() {
            self.remove_listener();
        }

        let duration = self.playback_ctx
            .as_ref()
            .unwrap()
            .info
            .lock()
            .unwrap()
            .duration;
        self.duration = duration;
    }

    fn export(&mut self) {
        let (format, export_type) = self.get_export_selection();

        self.prepare_process(&format);

        match export_type {
            ExportType::ExternalToc => {
                // export toc as a standalone file
                let (msg_type, msg) = match File::create(&self.target_path) {
                    Ok(mut output_file) => {
                        let info = self.playback_ctx.as_ref().unwrap().info.lock().unwrap();
                        match metadata::Factory::get_writer(&format).write(&info, &mut output_file)
                        {
                            Ok(_) => (
                                gtk::MessageType::Info,
                                gettext("Table of contents exported succesfully"),
                            ),
                            Err(err) => (gtk::MessageType::Error, err),
                        }
                    }
                    Err(_) => (
                        gtk::MessageType::Error,
                        gettext("Failed to create the file for the table of contents"),
                    ),
                };

                self.restore_context();
                self.switch_to_available();
                self.show_message(msg_type, &msg);
            }
            ExportType::SingleFileWithToc => {
                let target_path = self.target_path.clone();
                self.build_toc_setter_context(&target_path);
            }
        }
    }

    fn split(&mut self) {
        let format = self.get_split_selection();

        self.prepare_process(&format);

        if self.toc_visitor.is_some() {
            if let Err(err) = self.build_splitter_context(format) {
                self.show_error(&err);
            }
        } else {
            // No chapter => export the whole file
            let target_path = self.target_path.clone();
            self.build_toc_setter_context(&target_path);
        }
    }

    fn show_message(&self, type_: gtk::MessageType, message: &str) {
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        main_ctrl_rc.borrow().show_message(type_, message);
    }

    fn show_info(&self, info: &str) {
        self.show_message(gtk::MessageType::Info, info);
    }

    fn show_error(&self, error: &str) {
        self.show_message(gtk::MessageType::Error, error);
    }

    fn restore_context(&mut self) {
        let context = self.playback_ctx.take().unwrap();
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        main_ctrl_rc.borrow_mut().set_context(context);
    }

    fn build_toc_setter_context(&mut self, export_path: &Path) {
        let (ctx_tx, ui_rx) = channel();

        self.register_toc_setter_listener(LISTENER_PERIOD, ui_rx);

        let media_path = self.media_path.clone();
        match TocSetterContext::new(&media_path, export_path, ctx_tx) {
            Ok(toc_setter_ctx) => {
                self.switch_to_busy();
                self.toc_setter_ctx = Some(toc_setter_ctx);
            }
            Err(error) => {
                self.remove_listener();
                self.switch_to_available();
                self.restore_context();
                let msg = gettext("Failed to prepare for export. {}").replacen("{}", &error, 1);
                self.show_error(&msg);
                error!("{}", msg);
            }
        };
    }

    fn build_splitter_context(&mut self, format: metadata::Format) -> Result<bool, String> {
        let mut chapter = match self.next_chapter() {
            Some(chapter) => chapter,
            None => return Ok(false),
        };
        // Unfortunately, we need to make a copy here
        // because the chapter is also owned by the self.toc
        // and the TocVisitor so the chapters entries ref_count is > 1
        let chapter = self.update_tags(&mut chapter);

        let output_path = self.get_split_path(&chapter);
        let media_path = self.media_path.clone();

        let (ctx_tx, ui_rx) = channel();
        self.register_splitter_listener(format, LISTENER_PERIOD, ui_rx);
        match SplitterContext::new(&media_path, &output_path, format, chapter, ctx_tx) {
            Ok(splitter_ctx) => {
                self.switch_to_busy();
                self.splitter_ctx = Some(splitter_ctx);
                Ok(true)
            }
            Err(error) => {
                self.remove_listener();
                self.switch_to_available();
                self.restore_context();
                let msg = gettext("Failed to prepare for split. {}").replacen("{}", &error, 1);
                error!("{}", msg);
                Err(msg)
            }
        }
    }

    fn next_chapter(&mut self) -> Option<gst::TocEntry> {
        let chapter = self.toc_visitor.as_mut().unwrap().next_chapter();

        if chapter.is_some() {
            self.idx += 1;
        }

        chapter
    }

    fn get_split_path(&self, chapter: &gst::TocEntry) -> PathBuf {
        let mut split_name = String::new();

        let info = self.playback_ctx.as_ref().unwrap().info.lock().unwrap();

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
            .unwrap_or_else(|| get_default_chapter_title());

        split_name += &format!("{:02}. {}.{}", self.idx, &track_title, self.extension,);

        self.target_path.with_file_name(split_name)
    }

    #[cfg_attr(feature = "cargo-clippy", allow(cyclomatic_complexity))]
    fn update_tags(&self, chapter: &mut gst::TocEntry) -> gst::TocEntry {
        let mut tags = gst::TagList::new();
        {
            let tags = tags.get_mut().unwrap();
            let chapter_count = {
                let info = self.playback_ctx.as_ref().unwrap().info.lock().unwrap();

                // Select tags suitable for a track
                add_tag_from!(tags, info.tags, gst::tags::Artist);
                add_tag_from!(tags, info.tags, gst::tags::ArtistSortname);
                add_tag_from!(tags, info.tags, gst::tags::Album);
                add_tag_from!(tags, info.tags, gst::tags::AlbumSortname);
                add_tag_from!(tags, info.tags, gst::tags::AlbumArtist);
                add_tag_from!(tags, info.tags, gst::tags::AlbumArtistSortname);
                add_tag_from!(tags, info.tags, gst::tags::Date);
                add_tag_from!(tags, info.tags, gst::tags::DateTime);
                add_tag_from!(tags, info.tags, gst::tags::Genre);
                add_tag_from!(tags, info.tags, gst::tags::Comment);
                add_tag_from!(tags, info.tags, gst::tags::ExtendedComment);
                add_tag_from!(tags, info.tags, gst::tags::AlbumVolumeNumber);
                add_tag_from!(tags, info.tags, gst::tags::AlbumVolumeCount);
                add_tag_from!(tags, info.tags, gst::tags::Location);
                add_tag_from!(tags, info.tags, gst::tags::Homepage);
                add_tag_from!(tags, info.tags, gst::tags::Description);
                add_tag_from!(tags, info.tags, gst::tags::Version);
                add_tag_from!(tags, info.tags, gst::tags::ISRC);
                add_tag_from!(tags, info.tags, gst::tags::Organization);
                add_tag_from!(tags, info.tags, gst::tags::Copyright);
                add_tag_from!(tags, info.tags, gst::tags::CopyrightUri);
                add_tag_from!(tags, info.tags, gst::tags::Composer);
                add_tag_from!(tags, info.tags, gst::tags::Conductor);
                add_tag_from!(tags, info.tags, gst::tags::Contact);
                add_tag_from!(tags, info.tags, gst::tags::License);
                add_tag_from!(tags, info.tags, gst::tags::LicenseUri);
                add_tag_from!(tags, info.tags, gst::tags::Performer);
                add_tag_from!(tags, info.tags, gst::tags::Contact);
                add_tag_from!(tags, info.tags, gst::tags::AlbumGain);
                add_tag_from!(tags, info.tags, gst::tags::AlbumPeak);
                add_tag_from!(tags, info.tags, gst::tags::ReferenceLevel);
                add_tag_from!(tags, info.tags, gst::tags::LanguageCode);
                add_tag_from!(tags, info.tags, gst::tags::LanguageName);
                add_tag_from!(tags, info.tags, gst::tags::BeatsPerMinute);
                add_tag_from!(tags, info.tags, gst::tags::Keywords);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationName);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationLatitude);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationLongitute);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationElevation);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationCity);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationCountry);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationSublocation);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationHorizontalError);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationMovementDirection);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationMovementSpeed);
                add_tag_from!(tags, info.tags, gst::tags::GeoLocationCaptureDirection);
                add_tag_from!(tags, info.tags, gst::tags::ShowName);
                add_tag_from!(tags, info.tags, gst::tags::ShowSortname);
                add_tag_from!(tags, info.tags, gst::tags::ShowEpisodeNumber);
                add_tag_from!(tags, info.tags, gst::tags::ShowSeasonNumber);
                add_tag_from!(tags, info.tags, gst::tags::ComposerSortname);
                add_tag_from!(tags, info.tags, gst::tags::Publisher);
                add_tag_from!(tags, info.tags, gst::tags::InterpretedBy);
                add_tag_from!(tags, info.tags, gst::tags::PrivateData);

                for image_iter in info.tags.iter_tag::<gst::tags::Image>() {
                    tags.add::<gst::tags::Image>(
                        image_iter.get().as_ref().unwrap(),
                        gst::TagMergeMode::Append,
                    );
                }

                info.chapter_count.unwrap()
            };

            // Add track specific tags
            let title = chapter
                .get_tags()
                .and_then(|tags| {
                    tags.get::<gst::tags::Title>()
                        .map(|tag| tag.get().unwrap().to_owned())
                })
                .unwrap_or_else(|| get_default_chapter_title());
            tags.add::<gst::tags::Title>(&title.as_str(), gst::TagMergeMode::Replace);

            let (start, end) = chapter.get_start_stop_times().unwrap();

            tags.add::<gst::tags::TrackNumber>(&(self.idx as u32), gst::TagMergeMode::Replace);
            tags.add::<gst::tags::TrackCount>(&(chapter_count as u32), gst::TagMergeMode::Replace);
            tags.add::<gst::tags::Duration>(
                &gst::ClockTime::from_nseconds((end - start) as u64),
                gst::TagMergeMode::Replace,
            );
            tags.add::<gst::tags::ApplicationName>(&"media-toc", gst::TagMergeMode::Replace);
        }

        let chapter = chapter.make_mut();
        chapter.set_tags(tags);
        chapter.to_owned()
    }

    fn get_export_selection(&self) -> (metadata::Format, ExportType) {
        if self.mkvmerge_txt_row.is_selected() {
            (Format::MKVMergeText, ExportType::ExternalToc)
        } else if self.cue_row.is_selected() {
            (Format::CueSheet, ExportType::ExternalToc)
        } else if self.mkv_row.is_selected() {
            (Format::Matroska, ExportType::SingleFileWithToc)
        } else {
            unreachable!("ExportController::get_export_selection unknown export type");
        }
    }

    fn get_split_selection(&self) -> metadata::Format {
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
        // TODO: allow cancelling export / split
        if let Some(main_ctrl) = self.main_ctrl.as_ref().unwrap().upgrade() {
            main_ctrl.borrow().set_cursor_waiting();
        }

        self.perspective_selector.set_sensitive(false);
        self.open_btn.set_sensitive(false);
        self.chapter_grid.set_sensitive(false);

        self.export_list.set_sensitive(false);
        self.export_btn.set_sensitive(false);
        self.split_list.set_sensitive(false);
        self.split_btn.set_sensitive(false);
    }

    fn switch_to_available(&self) {
        if let Some(main_ctrl) = self.main_ctrl.as_ref().unwrap().upgrade() {
            main_ctrl.borrow().reset_cursor();
        }

        self.export_progress_bar.set_fraction(0f64);
        self.split_progress_bar.set_fraction(0f64);

        self.perspective_selector.set_sensitive(true);
        self.open_btn.set_sensitive(true);
        self.chapter_grid.set_sensitive(true);

        self.export_list.set_sensitive(true);
        self.export_btn.set_sensitive(true);
        self.split_btn.set_sensitive(true);
        self.split_list.set_sensitive(true);
    }

    fn remove_listener(&mut self) {
        if let Some(source_id) = self.listener_src.take() {
            glib::source_remove(source_id);
        }
    }

    fn register_toc_setter_listener(&mut self, timeout: u32, ui_rx: Receiver<ContextMessage>) {
        let this_rc = Rc::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;

            let mut this = this_rc.borrow_mut();

            if this.duration > 0 {
                let position = match this.toc_setter_ctx.as_mut() {
                    Some(toc_setter_ctx) => toc_setter_ctx.get_position(),
                    None => 0,
                };
                this.export_progress_bar
                    .set_fraction(position as f64 / this.duration as f64);
            }

            for message in ui_rx.try_iter() {
                match message {
                    InitDone => {
                        let mut toc_setter_ctx = this.toc_setter_ctx.take().unwrap();

                        let exporter = MatroskaTocFormat::new();
                        {
                            let muxer = toc_setter_ctx.get_muxer().unwrap();
                            let info = this.playback_ctx.as_ref().unwrap().info.lock().unwrap();
                            exporter.export(&info, muxer);
                        }

                        match toc_setter_ctx.export() {
                            Ok(_) => (),
                            Err(err) => {
                                keep_going = false;
                                let message =
                                    gettext("Failed to export media. {}").replacen("{}", &err, 1);
                                this.show_error(&message);
                                error!("{}", message);
                            }
                        }

                        this.toc_setter_ctx = Some(toc_setter_ctx);
                    }
                    Eos => {
                        this.show_info(&gettext("Media exported succesfully"));
                        keep_going = false;
                    }
                    FailedToExport(error) => {
                        keep_going = false;
                        let message =
                            gettext("Failed to export media. {}").replacen("{}", &error, 1);
                        this.show_error(&message);
                        error!("{}", message);
                    }
                    _ => (),
                };

                if !keep_going {
                    break;
                }
            }

            if !keep_going {
                this.listener_src = None;
                this.switch_to_available();
                this.restore_context();
            }

            glib::Continue(keep_going)
        }));
    }

    fn register_splitter_listener(
        &mut self,
        format: metadata::Format,
        timeout: u32,
        ui_rx: Receiver<ContextMessage>,
    ) {
        let this_rc = Rc::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;
            let mut process_done = false;

            let mut this = this_rc.borrow_mut();

            if this.duration > 0 {
                let position = match this.splitter_ctx.as_mut() {
                    Some(splitter_ctx) => splitter_ctx.get_position(),
                    None => 0,
                };
                this.split_progress_bar
                    .set_fraction(position as f64 / this.duration as f64);
            }

            for message in ui_rx.try_iter() {
                match message {
                    Eos => {
                        process_done = match this.build_splitter_context(format) {
                            Ok(true) => false, // more chapters
                            Ok(false) => {
                                this.show_info(&gettext("Media split succesfully"));
                                true
                            }
                            Err(err) => {
                                this.show_error(&err);
                                true
                            }
                        };

                        keep_going = false;
                    }
                    FailedToExport(error) => {
                        this.listener_src = None;
                        keep_going = false;
                        process_done = true;
                        let message =
                            gettext("Failed to split media. {}").replacen("{}", &error, 1);
                        error!("{}", message);
                        this.show_error(&message);
                    }
                    _ => (),
                };

                if !keep_going {
                    break;
                }
            }

            if !keep_going && process_done {
                this.switch_to_available();
                this.restore_context();
            }

            glib::Continue(keep_going)
        }));
    }
}
