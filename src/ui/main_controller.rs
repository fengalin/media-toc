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

    fn process_message(&mut self,
        ui_tx: Option<&Sender<ContextMessage>>,
        message: ContextMessage
    ) -> bool
    {
        let keep_going = match message {
            Eos => {
                println!("Received Eos");
                self.ctx.as_ref()
                    .expect("Received Eos, but context is not available")
                    .stop();
                true
            },
            FailedToOpenMedia => {
                println!("ERROR: failed to open media");
                self.ctx = None;
                // TODO: clear UI
                false
            },
            OpenedMedia(info) => {
                println!("Received OpenedMedia");

                let ref context = self.ctx.as_ref()
                    .expect("Received OpenedMedia, but context is not available");

                self.info_ctrl.new_media(context, &info);
                self.video_ctrl.new_media(context, &info);
                self.audio_ctrl.new_media(context, &info);

                self.header_bar.set_subtitle(Some(context.file_name.as_str()));
                //context.pause();
                true
            },
            HaveVideoWidget(video_widget) => {
                let ui_tx = ui_tx.as_ref()
                    .expect("Received HaveVideoWidget, but no ui_tx is defined");
                self.video_ctrl.have_widget(video_widget);
                ui_tx.send(GotVideoWidget)
                    .expect("Failed to send GotVideoWidget to context");
                true
            },
            _ => false,
        };

        keep_going
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
        ui_tx: Option<Sender<ContextMessage>>,
        ui_rx: Receiver<ContextMessage>,
        timeout: u32
    )
    {
        if let Some(ref self_weak) = self.self_weak {
            let self_weak = self_weak.clone();

            gtk::timeout_add(timeout, move || {
                let mut keep_going = false;
                match ui_rx.try_recv() {
                    Ok(message) => match self_weak.upgrade() {
                        Some(mc) => keep_going = mc.borrow_mut().process_message(ui_tx.as_ref(), message),
                        None => panic!("Main controller is no longer available for ctx channel listener"),
                    },
                    Err(error) => match error {
                        TryRecvError::Empty => keep_going = true,
                        error => println!("Error listening to ctx channel: {:?}", error),
                    },
                };

                glib::Continue(keep_going)
            });
        }
        // FIXME: else use proper expression to panic!
    }

    fn open_media(&mut self, filepath: PathBuf) {
        if let Some(context) = self.ctx.as_ref() {
            context.stop();
        }

        let (ctx_tx, ui_rx) = channel();
        let (ui_tx, ctx_rx) = channel();

        self.register_listener(Some(ui_tx), ui_rx, 100);

        match Context::open_media_path(filepath, ctx_tx, ctx_rx) {
            Ok(ctx) => {
                self.ctx = Some(ctx);
            },
            Err(error) => println!("Error opening media: {}", error),
        };
    }
}
