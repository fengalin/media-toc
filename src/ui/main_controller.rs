extern crate gtk;
extern crate glib;

use std::rc::{Rc, Weak};
use std::cell::RefCell;

use std::path::PathBuf;

use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};

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

    ctx: Option<Context>,
    pending_ctx: Option<Context>,

    self_weak: Option<Weak<RefCell<MainController>>>,
}

impl MainController {
    pub fn new(builder: gtk::Builder) -> Rc<RefCell<Self>> {
        let mc = Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").unwrap(),
            header_bar: builder.get_object("header-bar").unwrap(),
            info_ctrl: InfoController::new(&builder),
            video_ctrl: VideoController::new(&builder),
            audio_ctrl: AudioController::new(&builder),
            ctx: None,
            pending_ctx: None,
            self_weak: None,
        }));

        {
            // TODO: stop any playing context or pending_ctx
            let mut mc_mut = mc.borrow_mut();
            mc_mut.window.connect_delete_event(|_, _| {
                gtk::main_quit();
                Inhibit(false)
            });
            mc_mut.window.set_titlebar(&mc_mut.header_bar);

            let mc_weak = Rc::downgrade(&mc);
            mc_mut.self_weak = Some(mc_weak);
        }

        let open_btn: Button = builder.get_object("open-btn").unwrap();
        let mc_weak = Rc::downgrade(&mc);
        open_btn.connect_clicked(move |_|
            match mc_weak.upgrade() {
                Some(mc) => mc.borrow_mut().select_media(),
                None => panic!("Main controller is no longer available for select_media"),
            }
        );

        mc
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }

    fn process_message(&mut self, message: ContextMessage) -> bool
    {
        let wait_for_more = match message {
            OpenedMedia => {
                println!("Processing OpenedMedia");

                let context = self.pending_ctx.take()
                    .expect("Received OpenedMedia, but new context is not available");

                self.info_ctrl.new_media(&context);
                self.video_ctrl.new_media(&context);
                self.audio_ctrl.new_media(&context);

                self.header_bar.set_subtitle(Some(context.file_name.as_str()));

                self.ctx = Some(context);

                false
            },
            FailedToOpenMedia => {
                println!("ERROR: failed to open media");
                self.pending_ctx = None;
                false
            },
            _ => false,
        };

        wait_for_more
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

    fn register_listener(&self,
        ui_rx: Receiver<ContextMessage>,
        timeout: u32
    )
    {
        if let Some(ref self_weak) = self.self_weak {
            let self_weak = self_weak.clone();

            gtk::timeout_add(timeout, move || {
                let mut wait_for_more = false;
                match ui_rx.try_recv() {
                    Ok(message) => match self_weak.upgrade() {
                        Some(mc) => wait_for_more = mc.borrow_mut().process_message(message),
                        None => panic!("Main controller is no longer available for ctx channel listener"),
                    },
                    Err(error) => match error {
                        TryRecvError::Empty => wait_for_more = true,
                        error => println!("Error listening to ctx channel: {:?}", error),
                    },
                };

                glib::Continue(wait_for_more)
            });
        }
        // FIXME: else use proper expression to panic! in
    }

    fn open_media(&mut self, filepath: PathBuf) {
        let (ctx_tx, ui_rx) = channel();

        self.register_listener(ui_rx, 100);

        match Context::open_media_path(filepath, &self.video_ctrl.video_box, ctx_tx) {
            Ok(ctx) => {
                self.pending_ctx = Some(ctx);
                let ref ctx = self.pending_ctx.as_ref().unwrap();
                ctx.play();
            },
            Err(error) => println!("Error opening media: {}", error),
        };
    }
}
