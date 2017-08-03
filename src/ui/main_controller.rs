extern crate gtk;
extern crate glib;

use std::rc::Rc;
use std::cell::{Ref, RefCell};

use std::thread;

use std::path::PathBuf;

use std::sync::mpsc::{channel, Receiver, Sender};

use gtk::prelude::*;
use gtk::{ApplicationWindow, HeaderBar, Button,
          FileChooserDialog, ResponseType, FileChooserAction};

use ::media::{Context, ContextMessage};
use ::media::ContextMessage::*;

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
}

impl MainController {
    pub fn new(builder: gtk::Builder) -> Rc<RefCell<Self>> {
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
                Some(mc) => mc.borrow_mut().select_media(),
                None => panic!("Main controller is no longer available for select_media"),
            }
        );

        let mc_weak = Rc::downgrade(&mc);
        gtk::timeout_add(200, move || {
            match ctx_rx.try_recv() {
                Ok(message) => match mc_weak.upgrade() {
                    Some(mc) => mc.borrow_mut().process_message(message),
                    None => panic!("Main controller is no longer available for ctx channel listener"),
                },
                Err(error) => match error {
                    Empty => (),
                    error => println!("Error listening to ctx channel: {:?}", error),
                },
            };

            glib::Continue(true)
        });

        mc
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }

    fn process_message(&mut self, message: ContextMessage) {
        println!("Processing message");
        match message {
            OpenedMedia(context) => {
                self.header_bar.set_subtitle(Some(context.file_name.as_str()));

                self.info_ctrl.new_media(&context);
                self.video_ctrl.new_media(&context);
                self.audio_ctrl.new_media(&context);

                self.context = Some(context);
            },
            FailedToOpenMedia => println!("ERROR: failed to open media"),
            _ => (),
        };
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
        let ctx_tx = self.ctx_tx.clone();
        thread::spawn(move || {
            Context::open_media_path_thread(filepath, ctx_tx);
        });

        // FIXME: this hangs pipeline's pad_added gtksink creation
        // has it blocks the GTK thread
        // for the moment, let's use an ugly timeout_add defined at main controller's creation
        /*match ctx_rx.recv() {
            Ok(message) => self.process_message(message),
            // TODO: handle cases (Empty, Error...)
            Err(error) => println!("ctx_rx: {:?}", error),
        };*/
    }
}
