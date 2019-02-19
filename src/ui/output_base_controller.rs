use glib;

use gtk;
use gtk::prelude::*;

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::{Rc, Weak},
};

use crate::{media::PlaybackContext, metadata};

use super::MainController;

pub struct OutputBaseController {
    perspective_selector: gtk::MenuButton,
    open_btn: gtk::Button,
    chapter_grid: gtk::Grid,

    pub playback_ctx: Option<PlaybackContext>,
    pub media_path: PathBuf,
    pub target_path: PathBuf,
    pub extension: String,
    pub duration: u64,

    pub listener_src: Option<glib::SourceId>,
    main_ctrl: Option<Weak<RefCell<MainController>>>,
}

impl OutputBaseController {
    pub fn new(builder: &gtk::Builder) -> Self {
        OutputBaseController {
            perspective_selector: builder.get_object("perspective-menu-btn").unwrap(),
            open_btn: builder.get_object("open-btn").unwrap(),
            chapter_grid: builder.get_object("info-chapter_list-grid").unwrap(),

            playback_ctx: None,
            media_path: PathBuf::new(),
            target_path: PathBuf::new(),
            extension: String::new(),
            duration: 0,

            listener_src: None,
            main_ctrl: None,
        }
    }

    pub fn have_main_ctrl(&mut self, main_ctrl: &Rc<RefCell<MainController>>) {
        self.main_ctrl = Some(Rc::downgrade(main_ctrl));
    }

    pub fn prepare_process(&mut self, format: metadata::Format, is_audio_only: bool) {
        self.switch_to_busy();

        self.extension = metadata::Factory::get_extension(format, is_audio_only).to_owned();

        if self.listener_src.is_some() {
            self.remove_listener();
        }

        {
            let info = self.playback_ctx.as_ref().unwrap().info.read().unwrap();
            self.media_path = info.path.clone();
            self.duration = info.duration;
        }
        self.target_path = self.media_path.with_extension(&self.extension);
    }

    pub fn show_message<Msg: AsRef<str>>(&self, type_: gtk::MessageType, message: Msg) {
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        main_ctrl_rc.borrow().show_message(type_, message);
    }

    pub fn show_info<Msg: AsRef<str>>(&self, info: Msg) {
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        main_ctrl_rc.borrow().show_info(info);
    }

    pub fn show_error<Msg: AsRef<str>>(&self, error: Msg) {
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        main_ctrl_rc.borrow().show_error(error);
    }

    pub fn restore_context(&mut self) {
        let context = self.playback_ctx.take().unwrap();
        let main_ctrl_rc = self.main_ctrl.as_ref().unwrap().upgrade().unwrap();
        main_ctrl_rc.borrow_mut().set_context(context);
    }

    pub fn switch_to_busy(&self) {
        if let Some(main_ctrl) = self.main_ctrl.as_ref().unwrap().upgrade() {
            main_ctrl.borrow().set_cursor_waiting();
        }

        self.perspective_selector.set_sensitive(false);
        self.open_btn.set_sensitive(false);
        self.chapter_grid.set_sensitive(false);
    }

    pub fn switch_to_available(&self) {
        if let Some(main_ctrl) = self.main_ctrl.as_ref().unwrap().upgrade() {
            main_ctrl.borrow().reset_cursor();
        }

        self.perspective_selector.set_sensitive(true);
        self.open_btn.set_sensitive(true);
        self.chapter_grid.set_sensitive(true);
    }

    pub fn remove_listener(&mut self) {
        if let Some(source_id) = self.listener_src.take() {
            glib::source_remove(source_id);
        }
    }
}
