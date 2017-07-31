extern crate gtk;

use std::rc::Rc;
use std::cell::RefCell;

use std::path::PathBuf;

use std::sync::mpsc::{channel, Receiver, Sender};

use gtk::prelude::*;
use gtk::{ApplicationWindow, HeaderBar, Button,
          FileChooserDialog, ResponseType, FileChooserAction};

use ::media::{Context, ContextMessage};

use super::{AudioController, InfoController, MediaHandler, VideoController};

pub struct MainController {
    window: ApplicationWindow,
    header_bar: HeaderBar,
    info_ctrl: InfoController,
    video_ctrl: VideoController,
    audio_ctrl: AudioController,

    filepath: PathBuf,
    context: Option<Context>,

    ctx_tx: Sender<ContextMessage>,
    ctx_rx: Receiver<ContextMessage>,
}

impl MainController {
    pub fn new(builder: gtk::Builder) -> Rc<RefCell<MainController>> {
        let (ctx_tx, ctx_rx) = channel::<ContextMessage>();

        let mc = Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").unwrap(),
            header_bar: builder.get_object("header-bar").unwrap(),
            info_ctrl: InfoController::new(&builder),
            video_ctrl: VideoController::new(&builder),
            audio_ctrl: AudioController::new(&builder),
            filepath: PathBuf::new(),
            context: None,
            ctx_tx: ctx_tx,
            ctx_rx: ctx_rx,
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
        let mc_weak = Rc::downgrade(&mc);
        open_btn.connect_clicked(move |_|
            match mc_weak.upgrade() {
                Some(main_controller) => main_controller.borrow_mut().select_media(),
                None => panic!("Main controller is no longer available for select_media"),
            }
        );

        mc
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
        else {
            ()
        }

        file_dlg.close();
    }

    fn open_media(&mut self, filepath: PathBuf) {
        self.filepath = filepath;

        let path_str = String::from(self.filepath.to_str().unwrap());
        // TODO: restore hanlders arguments (and manage lifetimes)
        let message = match Context::new(
            &self.filepath,
            self.ctx_tx.clone(),
            &self.video_ctrl.drawingarea
        )
        {
            Ok(mut context) => {
                {
                    let file_name: &str = &context.file_name;
                    self.header_bar.set_subtitle(Some(file_name));
                }

                self.info_ctrl.new_media(&context);
                self.video_ctrl.new_media(&context);
                self.audio_ctrl.new_media(&context);

                self.context = Some(context);

                format!("Opened media {:?}", path_str)
            },
            Err(error) => format!("Error opening media {}, {}", path_str, error),
        };

        println!("{}", message);
    }
}
