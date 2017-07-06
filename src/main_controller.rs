extern crate gtk;

use std::rc::Rc;
use std::cell::RefCell;

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{ApplicationWindow, HeaderBar, Statusbar, Button,
          FileChooserDialog, ResponseType, FileChooserAction};

use controller_ext::Notifiable;
use video_controller::VideoController;
use audio_controller::AudioController;

pub struct MainController {
    window: ApplicationWindow,
    header_bar: HeaderBar,
    status_bar: Statusbar,
    video_ctrl: Rc<RefCell<VideoController>>,
    audio_ctrl: Rc<RefCell<AudioController>>,

    filename: PathBuf,
}

impl MainController {
    pub fn new(builder: gtk::Builder) -> Rc<RefCell<MainController>> {
        let mc = Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").unwrap(),
            header_bar: builder.get_object("header-bar").unwrap(),
            status_bar: builder.get_object("status-bar").unwrap(),
            video_ctrl: VideoController::new(&builder),
            audio_ctrl: AudioController::new(&builder),
            filename: PathBuf::new(),
        }));

        {
            let mc_ref = mc.borrow();
            mc_ref.window.connect_delete_event(|_, _| {
                gtk::main_quit();
                Inhibit(false)
            });
            mc_ref.window.set_titlebar(&mc_ref.header_bar);
            mc_ref.display_something();
        }

        let open_btn: Button = builder.get_object("open-btn").unwrap();
        let mc_for_cb = mc.clone();
        open_btn.connect_clicked(move |_| mc_for_cb.borrow_mut().select_media());

        mc
    }

    fn display_something(&self) {
        self.status_bar.push(self.status_bar.get_context_id("dummy msg"),
                             "Media-TOC prototype");
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }

    fn select_media(&mut self) {
        let file_dlg = FileChooserDialog::new(Some("Open a media file"),
                                              Some(&self.window),
                                              FileChooserAction::Open,
            );
        // Note: couldn't find equivalents for STOCK_OK
        file_dlg.add_button("Open", ResponseType::Ok.into());

        let result = file_dlg.run();

        // Note: couldn't find a way to coerce ResponseType to i32 in a match statement
        if result == ResponseType::Ok.into() {
            self.open_media(file_dlg.get_filename().unwrap());
        }
        // else: cancelled => do nothing TODO: I think there is a Rust way to express this

        file_dlg.close();
    }

    fn open_media(&mut self, filename: PathBuf) {
        self.filename = filename;
        self.status_bar.push(self.status_bar.get_context_id("open media"),
                             &format!("Opened {:?}", self.filename));

        // TODO: use FFMPEG to open the media

        self.video_ctrl.borrow_mut().notify_new_media();
        self.audio_ctrl.borrow_mut().notify_new_media();
    }
}
