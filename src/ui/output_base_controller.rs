use glib;

use gtk;
use gtk::prelude::*;
use log::error;

use std::{
    cell::RefCell,
    collections::HashSet,
    path::PathBuf,
    rc::{Rc, Weak},
    sync::{Arc, RwLock},
};

use crate::{
    media::{MediaEvent, PlaybackPipeline},
    metadata,
    metadata::{Format, MediaInfo},
};

use super::{MainController, UIController};

const PROGRESS_TIMER_PERIOD: u32 = 250; // 250 ms

pub enum ProcessingType {
    Async(glib::Receiver<MediaEvent>),
    Sync,
}

pub enum ProcessingStatus {
    Completed(String),
    InProgress,
}

pub trait MediaProcessor {
    fn init(&mut self) -> ProcessingType;
    fn start(&mut self) -> Result<ProcessingStatus, String>;
    fn handle_media_event(&mut self, event: MediaEvent) -> Result<ProcessingStatus, String>;
    fn report_progress(&mut self) -> f64;
}

pub trait OutputControllerImpl {
    const BTN_NAME: &'static str;
    const LIST_NAME: &'static str;
    const PROGRESS_BAR_NAME: &'static str;
}

pub struct OutputMediaFileInfo {
    pub format: Format,
    pub path: PathBuf,
    pub extension: String,
    pub stream_ids: Arc<RwLock<HashSet<String>>>,
}

impl OutputMediaFileInfo {
    pub fn new(format: Format, src_info: &MediaInfo) -> Self {
        let (stream_ids, content) = src_info.get_stream_ids_to_export(format);
        let extension = metadata::Factory::get_extension(format, content).to_owned();

        OutputMediaFileInfo {
            path: src_info.path.with_extension(&extension),
            extension,
            format,
            stream_ids: Arc::new(RwLock::new(stream_ids)),
        }
    }
}

pub struct OutputBaseController<Impl> {
    impl_: Impl,

    progress_bar: gtk::ProgressBar,
    list: gtk::ListBox,
    btn: gtk::Button,

    perspective_selector: gtk::MenuButton,
    open_btn: gtk::Button,
    chapter_grid: gtk::Grid,

    pub playback_pipeline: Option<PlaybackPipeline>,

    pub progress_timer_src: Option<glib::SourceId>,

    this_opt: Option<Weak<RefCell<OutputBaseController<Impl>>>>,
    main_ctrl: Option<Weak<RefCell<MainController>>>,
}

impl<Impl: OutputControllerImpl + MediaProcessor + UIController + 'static>
    OutputBaseController<Impl>
{
    pub fn new_base_rc(impl_: Impl, builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(OutputBaseController {
            impl_,

            btn: builder.get_object(Impl::BTN_NAME).unwrap(),
            list: builder.get_object(Impl::LIST_NAME).unwrap(),
            progress_bar: builder.get_object(Impl::PROGRESS_BAR_NAME).unwrap(),

            perspective_selector: builder.get_object("perspective-menu-btn").unwrap(),
            open_btn: builder.get_object("open-btn").unwrap(),
            chapter_grid: builder.get_object("info-chapter_list-grid").unwrap(),

            playback_pipeline: None,

            progress_timer_src: None,

            this_opt: None,
            main_ctrl: None,
        }))
    }

    fn register_progress_timer(&mut self, period: u32) {
        debug_assert!(self.progress_timer_src.is_none());

        let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

        self.progress_timer_src = Some(glib::timeout_add_local(period, move || {
            let this_rc = this_weak
                .upgrade()
                .expect("Lost controller in progress timer");
            let mut this = this_rc.borrow_mut();
            let progress = this.impl_.report_progress();
            this.progress_bar.set_fraction(progress);

            glib::Continue(true)
        }));
    }

    fn register_media_event_handler(&mut self, receiver: glib::Receiver<MediaEvent>) {
        let this_weak = Weak::clone(self.this_opt.as_ref().unwrap());

        receiver.attach(None, move |event| {
            let this_rc = this_weak
                .upgrade()
                .expect("Lost controller in `MediaEvent` handler");
            let mut this = this_rc.borrow_mut();
            let is_in_progress = match this.impl_.handle_media_event(event) {
                Ok(ProcessingStatus::Completed(msg)) => {
                    this.show_info(msg);
                    false
                }
                Ok(ProcessingStatus::InProgress) => true,
                Err(err) => {
                    this.show_error(err);
                    false
                }
            };

            if is_in_progress {
                glib::Continue(true)
            } else {
                this.restore_pipeline();
                this.switch_to_available();
                glib::Continue(false)
            }
        });
    }

    pub fn have_main_ctrl(&mut self, main_ctrl: &Rc<RefCell<MainController>>) {
        self.main_ctrl = Some(Rc::downgrade(main_ctrl));
    }

    pub fn show_info<Msg: AsRef<str>>(&self, info: Msg) {
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        main_ctrl_rc.borrow().show_info(info);
    }

    pub fn show_error<Msg: AsRef<str>>(&self, error: Msg) {
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        main_ctrl_rc.borrow().show_error(error);
    }

    pub fn restore_pipeline(&mut self) {
        let playback_pipeline = self.playback_pipeline.take().unwrap();
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        main_ctrl_rc.borrow_mut().set_pipeline(playback_pipeline);
    }

    pub fn switch_to_busy(&self) {
        if let Some(main_ctrl) = self.main_ctrl.as_ref().unwrap().upgrade() {
            main_ctrl.borrow().set_cursor_waiting();
        }

        self.list.set_sensitive(false);
        self.btn.set_sensitive(false);

        self.perspective_selector.set_sensitive(false);
        self.open_btn.set_sensitive(false);
        self.chapter_grid.set_sensitive(false);
    }

    pub fn switch_to_available(&mut self) {
        self.remove_progress_timer();

        if let Some(main_ctrl) = self.main_ctrl.as_ref().unwrap().upgrade() {
            main_ctrl.borrow().reset_cursor();
        }

        self.progress_bar.set_fraction(0f64);
        self.list.set_sensitive(true);
        self.btn.set_sensitive(true);

        self.perspective_selector.set_sensitive(true);
        self.open_btn.set_sensitive(true);
        self.chapter_grid.set_sensitive(true);
    }

    fn remove_progress_timer(&mut self) {
        if let Some(src_id) = self.progress_timer_src.take() {
            let _res = glib::Source::remove(src_id);
        }
    }
}

impl<Impl: OutputControllerImpl + MediaProcessor + UIController + 'static> UIController
    for OutputBaseController<Impl>
{
    fn setup_(
        this_rc: &Rc<RefCell<Self>>,
        _gtk_app: &gtk::Application,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();
        this.have_main_ctrl(main_ctrl);

        this.this_opt = Some(Rc::downgrade(&this_rc));
        this.btn.set_sensitive(false);

        this.impl_.setup();

        let this_clone = Rc::clone(this_rc);
        let main_ctrl_clone = Rc::clone(main_ctrl);
        this.btn.connect_clicked(move |_| {
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
                        let mut this_mut = this_clone.borrow_mut();
                        this_mut.switch_to_busy();

                        match this_mut.impl_.init() {
                            ProcessingType::Sync => (),
                            ProcessingType::Async(receiver) => {
                                this_mut.register_media_event_handler(receiver);
                                this_mut.register_progress_timer(PROGRESS_TIMER_PERIOD);
                            }
                        }

                        let is_in_progress = match this_mut.impl_.start() {
                            Ok(ProcessingStatus::Completed(msg)) => {
                                this_mut.show_info(msg);
                                false
                            }
                            Ok(ProcessingStatus::InProgress) => true,
                            Err(err) => {
                                error!("{}", err);
                                this_mut.show_error(err);
                                false
                            }
                        };

                        if !is_in_progress {
                            this_mut.restore_pipeline();
                            this_mut.switch_to_available();
                        }

                        glib::Continue(false)
                    });
                }));
        });
    }

    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        self.btn.set_sensitive(true);
        self.impl_.new_media(pipeline);
    }

    fn cleanup(&mut self) {
        self.progress_bar.set_fraction(0f64);
        self.btn.set_sensitive(false);
        self.impl_.cleanup();
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.impl_.streams_changed(info);
    }
}
