use gettextrs::*;
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
use metadata::{Exporter, Format, MatroskaTocFormat, TocVisitor, DEFAULT_TITLE};

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
    None,
    SingleFileWithToc,
    Split,
}

pub struct ExportController {
    export_dlg: gtk::Dialog,
    open_export_btn: gtk::Button,
    export_btn: gtk::Button,
    progress_bar: gtk::ProgressBar,

    mkvmerge_txt_rdbtn: gtk::RadioButton,
    cue_rdbtn: gtk::RadioButton,
    mkv_rdbtn: gtk::RadioButton,
    split_rdbtn: gtk::RadioButton,
    split_to_flac_rdbtn: gtk::RadioButton,
    split_to_wave_rdbtn: gtk::RadioButton,
    split_to_opus_rdbtn: gtk::RadioButton,
    split_to_vorbis_rdbtn: gtk::RadioButton,
    split_to_mp3_rdbtn: gtk::RadioButton,

    pub playback_ctx: Option<PlaybackContext>,
    toc_setter_ctx: Option<TocSetterContext>,
    splitter_ctx: Option<SplitterContext>,
    export_format: metadata::Format,
    export_type: ExportType,
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
        let main_window: gtk::ApplicationWindow = builder.get_object("application-window").unwrap();
        let export_dlg: gtk::Dialog = builder.get_object("export-dlg").unwrap();
        export_dlg.set_transient_for(&main_window);

        let this = Rc::new(RefCell::new(ExportController {
            export_dlg,
            open_export_btn: builder.get_object("export_toc-btn").unwrap(),
            export_btn: builder.get_object("export-btn").unwrap(),
            progress_bar: builder.get_object("export-progress").unwrap(),

            mkvmerge_txt_rdbtn: builder.get_object("mkvmerge_txt-rdbtn").unwrap(),
            cue_rdbtn: builder.get_object("cue-rdbtn").unwrap(),
            mkv_rdbtn: builder.get_object("mkv-rdbtn").unwrap(),
            split_rdbtn: builder.get_object("split-rdbtn").unwrap(),
            split_to_flac_rdbtn: builder.get_object("split_to_flac-rdbtn").unwrap(),
            split_to_wave_rdbtn: builder.get_object("split_to_wave-rdbtn").unwrap(),
            split_to_opus_rdbtn: builder.get_object("split_to_opus-rdbtn").unwrap(),
            split_to_vorbis_rdbtn: builder.get_object("split_to_vorbis-rdbtn").unwrap(),
            split_to_mp3_rdbtn: builder.get_object("split_to_mp3-rdbtn").unwrap(),

            playback_ctx: None,
            toc_setter_ctx: None,
            splitter_ctx: None,
            export_format: Format::MKVMergeText,
            export_type: ExportType::None,
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

            this_mut.cleanup();

            // Set radio buttons initial availability
            this_mut.switch_to_available();
        }

        this
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();

        this.main_ctrl = Some(Rc::downgrade(main_ctrl));

        let this_clone = Rc::clone(this_rc);
        this.export_dlg.connect_delete_event(move |dlg, _| {
            this_clone.borrow_mut().restore_context();
            dlg.hide_on_delete();
            Inhibit(true)
        });

        let this_clone = Rc::clone(this_rc);
        this.split_rdbtn.connect_property_active_notify(move |_| {
            // Enable / disable split sub radio button depending on whether split is selected
            this_clone.borrow().set_split_sub_btn_sensitivity();
        });

        let this_clone = Rc::clone(this_rc);
        this.export_btn.connect_clicked(move |_| {
            let mut this = this_clone.borrow_mut();

            this.switch_to_available();

            let (format, export_type) = this.get_selected_format();
            this.export_format = format;
            this.export_type = export_type.clone();
            this.idx = 0;

            let is_audio_only = {
                this.playback_ctx
                    .as_ref()
                    .unwrap()
                    .info
                    .lock()
                    .expect("ExportController::export_btn clicked, failed to lock media info")
                    .streams
                    .video_selected
                    .is_none()
            };
            this.extension =
                metadata::Factory::get_extension(&this.export_format, is_audio_only).to_owned();

            this.media_path = this.playback_ctx.as_ref().unwrap().path.clone();
            this.target_path = this.media_path.with_extension(&this.extension);

            if this.listener_src.is_some() {
                this.remove_listener();
            }

            let duration = this.playback_ctx
                .as_ref()
                .unwrap()
                .info.lock()
                .expect("ExportController::export_btn clicked, failed to lock media info")
                .duration;
            this.duration = duration;

            match export_type {
                ExportType::ExternalToc => {
                    // export toc as a standalone file
                    let (msg_type, msg) = match File::create(&this.target_path) {
                        Ok(mut output_file) => {
                            let info = this.playback_ctx.as_ref().unwrap().info.lock().expect(
                                "ExportController::export_btn clicked, failed to lock media info",
                            );
                            metadata::Factory::get_writer(&this.export_format).write(
                                &info,
                                &mut output_file,
                            );
                            (
                                gtk::MessageType::Info,
                                gettext("Table of contents exported succesfully")
                                    .to_owned(),
                            )
                        }
                        Err(_) => (
                            gtk::MessageType::Error,
                            gettext("Failed to create the file for the table of contents")
                                .to_owned(),
                        ),
                    };

                    this.restore_context();
                    this.show_message(msg_type, &msg);
                }
                ExportType::Split => {
                    if this.toc_visitor.is_some() {
                        this.build_splitter_context();
                    } else {
                        // No chapter => export the whole file
                        let target_path = this.target_path.clone();
                        this.build_toc_setter_context(&target_path);
                    }
                }
                ExportType::SingleFileWithToc => {
                    let target_path = this.target_path.clone();
                    this.build_toc_setter_context(&target_path);
                }
                _ => (),
            }
        });
    }

    pub fn new_media(&mut self, _context: &PlaybackContext) {
        self.open_export_btn.set_sensitive(true);
    }

    pub fn cleanup(&mut self) {
        self.open_export_btn.set_sensitive(false);
    }

    pub fn open(&mut self, playback_ctx: PlaybackContext) {
        self.toc_visitor = playback_ctx
            .info
            .lock()
            .expect("ExportController::open failed to lock media info")
            .toc
                .as_ref()
                .map(|toc| {
                    TocVisitor::new(toc)
                });

        self.playback_ctx = Some(playback_ctx);
        self.progress_bar.set_fraction(0f64);
        self.export_dlg.present();
    }

    fn show_message(&self, type_: gtk::MessageType, message: &str) {
        self.export_dlg.hide();
        let main_ctrl_rc = self.main_ctrl
            .as_ref()
            .unwrap()
            .upgrade()
            .expect("ExportController::show_message can't upgrade main_ctrl");
        main_ctrl_rc.borrow().show_message(type_, message);
    }

    /*fn show_info(&self, info: &str) {
        self.show_message(gtk::MessageType::Info, info);
    }*/

    fn show_error(&self, error: &str) {
        self.show_message(gtk::MessageType::Error, error);
    }

    fn restore_context(&mut self) {
        let context = self.playback_ctx.take()
            .expect("ExportController::restore_context playback_ctx is None");
        let main_ctrl_rc = self.main_ctrl
            .as_ref()
            .unwrap()
            .upgrade()
            .expect("ExportController::restore_context can't upgrade main_ctrl");
        main_ctrl_rc
            .borrow_mut()
            .set_context(context);
    }

    fn build_toc_setter_context(
        &mut self,
        export_path: &Path,
    ) {
        let (ctx_tx, ui_rx) = channel();

        self.register_toc_setter_listener(LISTENER_PERIOD, ui_rx);

        let media_path = self.media_path.clone();
        match TocSetterContext::new(&media_path, export_path, ctx_tx) {
            Ok(toc_setter_ctx) => {
                self.switch_to_busy();
                self.toc_setter_ctx = Some(toc_setter_ctx);
            }
            Err(error) => {
                let msg = gettext("Error exporting the media with a table of contents").to_owned();
                eprintln!("{}: {}", msg, error);
                self.remove_listener();
                self.switch_to_available();
                self.restore_context();
                self.show_error(&msg);
            }
        };
    }

    fn build_splitter_context(&mut self) -> bool {
        let (ctx_tx, ui_rx) = channel();

        self.register_splitter_listener(LISTENER_PERIOD, ui_rx);

        if self.toc_visitor.is_none() {
            // FIXME: display message: no chapter
            return false;
        }

        let mut chapter = match self.next_chapter() {
            Some(chapter) => chapter,
            None => return false,
        };
        // Unfortunately, we need to make a copy here
        // because the chapter is also owned by the self.toc
        // and the TocVisitor so the chapters entries ref_count is > 1
        let chapter = self.update_tags(&mut chapter);

        let output_path = self.get_split_path(&chapter);

        let media_path = self.media_path.clone();
        match SplitterContext::new(
            &media_path,
            &output_path,
            &self.export_format,
            chapter,
            ctx_tx,
        ) {
            Ok(splitter_ctx) => {
                self.switch_to_busy();
                self.splitter_ctx = Some(splitter_ctx);
                true
            }
            Err(error) => {
                eprintln!("Error exporting media: {}", error);
                false
            }
        }
    }

    fn next_chapter(&mut self) -> Option<gst::TocEntry> {
        let chapter = self.toc_visitor
            .as_mut()
            .unwrap()
            .next_chapter();

        if chapter.is_some() {
            self.idx += 1;
        }

        chapter
    }

    fn get_split_path(&self, chapter: &gst::TocEntry) -> PathBuf {
        let mut split_name = String::new();

        let info = self.playback_ctx
            .as_ref()
            .unwrap()
            .info
            .lock()
            .expect("ExportController::get_split_name failed to lock media info");

        // TODO: make format customisable
        if let Some(artist) = info.get_artist() {
            split_name += &format!("{} - ", artist);
        }
        if let Some(album_title) = info.get_title() {
            split_name += &format!("{} - ", album_title);
        }

        let track_title = chapter.get_tags().map_or(None, |tags| {
            tags.get::<gst::tags::Title>().map(|tag| {
                tag.get().unwrap().to_owned()
            })
        }).unwrap_or(DEFAULT_TITLE.to_owned());

        split_name += &format!(
            "{:02}. {}.{}",
            self.idx,
            &track_title,
            self.extension,
        );

        self.target_path.with_file_name(split_name)
    }

    #[cfg_attr(feature = "cargo-clippy", allow(cyclomatic_complexity))]
    fn update_tags(&self, chapter: &mut gst::TocEntry) -> gst::TocEntry {
        let mut tags = gst::TagList::new();
        {
            let tags = tags.get_mut().unwrap();
            let chapter_count = {
                let info = self.playback_ctx
                    .as_ref()
                    .unwrap()
                    .info
                    .lock()
                    .expect("ExportController::update_tags failed to lock media info");

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

                info.chapter_count
                    .expect("ExportController::update_tags chapter_count is none")
            };

            // Add track specific tags
            let title = chapter.get_tags().map_or(None, |tags| {
                tags.get::<gst::tags::Title>().map(|tag| {
                    tag.get().unwrap().to_owned()
                })
            }).unwrap_or(DEFAULT_TITLE.to_owned());
            tags.add::<gst::tags::Title>(&title.as_str(), gst::TagMergeMode::Replace);

            let (start, end) = chapter
                .get_start_stop_times()
                .expect("SplitterContext::build_pipeline failed to get chapter's start/end");

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

    fn get_selected_format(&self) -> (metadata::Format, ExportType) {
        if self.mkvmerge_txt_rdbtn.get_active() {
            (Format::MKVMergeText, ExportType::ExternalToc)
        } else if self.cue_rdbtn.get_active() {
            (Format::CueSheet, ExportType::ExternalToc)
        } else if self.mkv_rdbtn.get_active() {
            (Format::Matroska, ExportType::SingleFileWithToc)
        } else if self.split_rdbtn.get_active() {
            if self.split_to_flac_rdbtn.get_active() {
                (Format::Flac, ExportType::Split)
            } else if self.split_to_wave_rdbtn.get_active() {
                (Format::Wave, ExportType::Split)
            } else if self.split_to_opus_rdbtn.get_active() {
                (Format::Opus, ExportType::Split)
            } else if self.split_to_vorbis_rdbtn.get_active() {
                (Format::Vorbis, ExportType::Split)
            } else if self.split_to_mp3_rdbtn.get_active() {
                (Format::MP3, ExportType::Split)
            } else {
                unreachable!("ExportController::get_selected_format no selected radio button");
            }
        } else {
            unreachable!("ExportController::get_selected_format no selected radio button");
        }
    }

    fn switch_to_busy(&self) {
        self.mkvmerge_txt_rdbtn.set_sensitive(false);
        self.cue_rdbtn.set_sensitive(false);
        self.mkv_rdbtn.set_sensitive(false);
        self.split_rdbtn.set_sensitive(false);
        self.export_btn.set_sensitive(false);
        self.split_to_flac_rdbtn.set_sensitive(false);
        self.split_to_wave_rdbtn.set_sensitive(false);
        self.split_to_opus_rdbtn.set_sensitive(false);
        self.split_to_vorbis_rdbtn.set_sensitive(false);
        self.split_to_mp3_rdbtn.set_sensitive(false);
    }

    fn switch_to_available(&self) {
        if TocSetterContext::check_requirements() {
            self.mkv_rdbtn.set_sensitive(true);
        } else {
            self.mkv_rdbtn.set_label("Matroska Container (requires gst-plugins-good >= 1.14)");
            self.mkv_rdbtn.set_sensitive(false);
        }

        self.mkvmerge_txt_rdbtn.set_sensitive(true);
        self.cue_rdbtn.set_sensitive(true);
        self.split_rdbtn.set_sensitive(true);
        self.set_split_sub_btn_sensitivity();

        self.export_btn.set_sensitive(true);
    }

    pub fn set_split_sub_btn_sensitivity(&self) {
        let state = self.split_rdbtn.get_active();
        self.split_to_flac_rdbtn.set_sensitive(
            state && SplitterContext::check_requirements(Format::Flac),
        );
        self.split_to_wave_rdbtn.set_sensitive(
            state && SplitterContext::check_requirements(Format::Wave),
        );
        self.split_to_opus_rdbtn.set_sensitive(
            state && SplitterContext::check_requirements(Format::Opus),
        );
        self.split_to_vorbis_rdbtn.set_sensitive(
            state && SplitterContext::check_requirements(Format::Vorbis),
        );
        self.split_to_mp3_rdbtn.set_sensitive(
            state && SplitterContext::check_requirements(Format::MP3),
        );
    }

    fn remove_listener(&mut self) {
        if let Some(source_id) = self.listener_src.take() {
            glib::source_remove(source_id);
        }
    }

    fn register_toc_setter_listener(
        &mut self,
        timeout: u32,
        ui_rx: Receiver<ContextMessage>,
    ) {
        let this_rc = Rc::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;

            let mut this = this_rc.borrow_mut();

            if this.duration > 0 {
                let position = match this.toc_setter_ctx.as_mut() {
                    Some(toc_setter_ctx) => toc_setter_ctx.get_position(),
                    None => 0,
                };
                this.progress_bar
                    .set_fraction(position as f64 / this.duration as f64);
            }

            for message in ui_rx.try_iter() {
                match message {
                    InitDone => {
                        let mut toc_setter_ctx = this.toc_setter_ctx.take().expect(
                            "ExportContext::toc_setter(InitDone) couldn't get ExportContext",
                        );

                        let exporter = MatroskaTocFormat::new();
                        {
                            let muxer = toc_setter_ctx
                                .get_muxer()
                                .expect("ExportContext::toc_setter(InitDone) couldn't get muxer");

                            let info = this.playback_ctx.as_ref().unwrap().info.lock().expect(
                                concat!(
                                    "ExportController::toc_setter(InitDone) ",
                                    "failed to lock media info",
                                ),
                            );
                            exporter.export(&info, muxer);
                        }

                        match toc_setter_ctx.export() {
                            Ok(_) => (),
                            Err(err) => {
                                this.switch_to_available();
                                eprintln!("ERROR: failed to export media: {}", err);
                            }
                        }

                        this.toc_setter_ctx = Some(toc_setter_ctx);
                    }
                    Eos => {
                        this.switch_to_available();
                        this.restore_context();
                        this.export_dlg.hide();
                        keep_going = false;
                    }
                    FailedToExport => {
                        this.switch_to_available();
                        eprintln!("ERROR: failed to export media");
                        keep_going = false;
                    }
                    _ => (),
                };

                if !keep_going {
                    break;
                }
            }

            if !keep_going {
                this.listener_src = None;
            }

            glib::Continue(keep_going)
        }));
    }

    fn register_splitter_listener(
        &mut self,
        timeout: u32,
        ui_rx: Receiver<ContextMessage>,
    ) {
        let this_rc = Rc::clone(self.this_opt.as_ref().unwrap());

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;

            let mut this = this_rc.borrow_mut();

            if this.duration > 0 {
                let position = match this.splitter_ctx.as_mut() {
                    Some(splitter_ctx) => splitter_ctx.get_position(),
                    None => 0,
                };
                this.progress_bar
                    .set_fraction(position as f64 / this.duration as f64);
            }

            for message in ui_rx.try_iter() {
                match message {
                    Eos => {
                        if !this.build_splitter_context() {
                            // No more chapters or an error occured
                            this.switch_to_available();
                            this.restore_context();
                            this.export_dlg.hide();
                        }

                        keep_going = false;
                    }
                    FailedToExport => {
                        this.switch_to_available();
                        eprintln!("ERROR: failed to export media");
                        keep_going = false;
                    }
                    _ => (),
                };

                if !keep_going {
                    break;
                }
            }

            if !keep_going {
                this.listener_src = None;
            }

            glib::Continue(keep_going)
        }));
    }
}
