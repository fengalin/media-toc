extern crate ffmpeg;

extern crate gtk;

use std::rc::Rc;
use std::cell::RefCell;

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{ApplicationWindow, HeaderBar, Statusbar, Button,
          FileChooserDialog, ResponseType, FileChooserAction};

use super::MediaNotifiable;
use super::VideoController;
use super::AudioController;

pub struct MainController {
    window: ApplicationWindow,
    header_bar: HeaderBar,
    status_bar: Statusbar,
    video_ctrl: Rc<RefCell<VideoController>>,
    audio_ctrl: Rc<RefCell<AudioController>>,

    filepath: PathBuf,
    context: Option<::media::context::Context>,
}

impl MainController {
    pub fn new(builder: gtk::Builder) -> Rc<RefCell<MainController>> {
        let mc = Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").unwrap(),
            header_bar: builder.get_object("header-bar").unwrap(),
            status_bar: builder.get_object("status-bar").unwrap(),
            video_ctrl: VideoController::new(&builder),
            audio_ctrl: AudioController::new(&builder),
            filepath: PathBuf::new(),
            context: None,
        }));

        {
            let mc_ref = mc.borrow();
            mc_ref.window.connect_delete_event(|_, _| {
                gtk::main_quit();
                Inhibit(false)
            });
            mc_ref.window.set_titlebar(&mc_ref.header_bar);
        }

        let open_btn: Button = builder.get_object("open-btn").unwrap();
        let mc_for_cb = mc.clone();
        open_btn.connect_clicked(move |_| mc_for_cb.borrow_mut().select_media());

        mc
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }


    fn display_message(&self, context: &str, message: &str) {
        self.status_bar.push(self.status_bar.get_context_id(context), message);
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

    fn open_media(&mut self, filepath: PathBuf) {
        self.filepath = filepath;

        let path_str = String::from(self.filepath.to_str().unwrap());
        let message = match ::media::Context::new(&self.filepath.as_path()) {
            Ok(context) => {
                self.context = Some(context);
                let context = self.context.as_mut().unwrap();

                self.video_ctrl.borrow_mut().new_media(context);
                match self.video_ctrl.borrow().stream_index() {
                    Some(stream_index) =>
                        context.register_packets(stream_index, self.video_ctrl.clone()),
                    None => (),
                }

                self.audio_ctrl.borrow_mut().new_media(context);
                match self.audio_ctrl.borrow().stream_index() {
                    Some(stream_index) =>
                        context.register_packets(stream_index, self.audio_ctrl.clone()),
                    None => (),
                }

                context.preview();

                format!("Opened media {:?}", path_str)
            },
            Err(error) => format!("Error opening media {}, {}", path_str, error),
        };

        self.display_message("open media", &message);
    }
}
