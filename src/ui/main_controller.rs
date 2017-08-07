extern crate gtk;
extern crate glib;

use std::rc::{Rc, Weak};
use std::cell::RefCell;

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
    audio_ctrl: Rc<RefCell<AudioController>>,

    ctx: Option<Context>,

    self_weak: Option<Weak<RefCell<MainController>>>,
    listener_src: Option<glib::SourceId>
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
            listener_src: None,
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
        open_btn.connect_clicked(move |_| {
            let mc = mc_weak.upgrade()
                .expect("Main controller is no longer available for select_media");
            mc.borrow_mut().select_media();
        });

        mc
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }

    pub fn stop(&mut self) {
        if let Some(context) = self.ctx.as_ref() {
            context.stop();
            if let Some(source_id) = self.listener_src {
                // remove listerner in order to avoid conflict on borrowing of self
                glib::source_remove(source_id);
            }
            self.listener_src = None;
        }
    }

    fn process_message(&mut self,
        message: ContextMessage,
        ui_tx_opt: Option<&Sender<ContextMessage>>
    ) -> bool
    {
        let mut keep_going = true;
        match message {
            AsyncDone => {
                println!("Received AsyncDone");
            },
            InitDone => {
                println!("Received InitDone");

                let ref context = self.ctx.as_ref()
                    .expect("Received InitDone, but context is not available");

                self.info_ctrl.new_media(context);
                self.video_ctrl.new_media(context);
                self.audio_ctrl.borrow_mut().new_media(context);

                self.header_bar.set_subtitle(Some(context.file_name.as_str()));
            },
            Eos => {
                println!("Received Eos");
                keep_going = false;
            },
            FailedToOpenMedia => {
                eprintln!("ERROR: failed to open media");
                self.ctx = None;
                // TODO: clear UI
                keep_going = false;
            },
            HaveAudioBuffer(buffer) => {
                self.audio_ctrl.borrow_mut().have_buffer(buffer);
                //println!("Received HaveAudioBuffer");
                /*println!("Received AudioBuffer with offset: {}, pts: {:?}, dts: {:?}, duration: {:?}",
                    buffer.get_offset(), buffer.get_pts(),
                    buffer.get_dts(), buffer.get_duration()
                );*/
            },
            HaveVideoWidget(video_widget) => {
                //println!("Received HaveVideoWidget");
                let ui_tx = ui_tx_opt.as_ref()
                    .expect("Received HaveVideoWidget, but no ui_tx is defined");
                self.video_ctrl.have_widget(video_widget);
                ui_tx.send(GotVideoWidget)
                    .expect("Failed to send GotVideoWidget to context");
            },
            _ => {
                eprintln!("Received an unexpected message => will quit listner");
                keep_going = false;
            },
        };

        if !keep_going {
            self.listener_src = None;
        }

        keep_going
    }

    fn select_media(&mut self) {
        self.stop();

        let file_dlg = FileChooserDialog::new(
            Some("Open a media file"),
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
        else { () }

        file_dlg.close();
    }

    fn register_listener(&mut self,
        ui_rx: Receiver<ContextMessage>,
        timeout: u32,
        ui_tx_opt: Option<Sender<ContextMessage>>,
    )
    {
        let self_weak = self.self_weak.as_ref()
            .expect("Failed to get ref on MainController's weak Rc for register_listener")
            .clone();

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;
            let mut msg_iter = ui_rx.try_iter();
            let first_msg_opt = msg_iter.next();
            if let Some(first_msg) = first_msg_opt {
                // only get mc as mut if a message is received
                let mc = self_weak.upgrade()
                    .expect("Main controller is no longer available for ctx channel listener");
                let mut mc_mut = mc.borrow_mut();
                keep_going = mc_mut.process_message(first_msg, ui_tx_opt.as_ref());
                if keep_going {
                    // process remaining messages
                    for msg in msg_iter {
                        keep_going = mc_mut.process_message(msg, ui_tx_opt.as_ref());
                        if !keep_going { break; }
                    }
                }
            };

            if !keep_going { println!("Exiting listener"); }
            glib::Continue(keep_going)
        }));
    }

    fn open_media(&mut self, filepath: PathBuf) {
        assert_eq!(self.listener_src, None);

        self.audio_ctrl.borrow_mut().clear();

        let (ctx_tx, ui_rx) = channel();
        let (ui_tx, ctx_rx) = channel();

        self.register_listener(ui_rx, 100, Some(ui_tx));

        match Context::open_media_path(filepath, ctx_tx, ctx_rx) {
            Ok(ctx) => self.ctx = Some(ctx),
            Err(error) => eprintln!("Error opening media: {}", error),
        };
    }
}
