extern crate gtk;
extern crate glib;
extern crate gstreamer as gst;

use std::rc::{Rc, Weak};
use std::cell::RefCell;

use std::path::PathBuf;

use std::sync::mpsc::{channel, Receiver, Sender};

use gtk::prelude::*;
use gtk::{ApplicationWindow, Button, FileChooserAction, FileChooserDialog,
          HeaderBar, ResponseType, Label, ToolButton};

use ::media::{Context, ContextMessage, Timestamp};
use ::media::ContextMessage::*;

use super::{AudioController, InfoController, VideoController};

pub struct MainController {
    window: ApplicationWindow,
    header_bar: HeaderBar,
    play_pause_btn: ToolButton,
    position_lbl: Label,
    info_ctrl: InfoController,
    video_ctrl: VideoController,
    audio_ctrl: Rc<RefCell<AudioController>>,

    ctx: Option<Context>,

    self_weak: Option<Weak<RefCell<MainController>>>,
    listener_src: Option<glib::SourceId>
}

impl MainController {
    pub fn new(builder: gtk::Builder) -> Rc<RefCell<Self>> {
        let this = Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").unwrap(),
            header_bar: builder.get_object("header-bar").unwrap(),
            play_pause_btn: builder.get_object("play_pause-toolbutton").unwrap(),
            position_lbl: builder.get_object("position-lbl").unwrap(),
            info_ctrl: InfoController::new(&builder),
            video_ctrl: VideoController::new(&builder),
            audio_ctrl: AudioController::new(&builder),
            ctx: None,
            self_weak: None,
            listener_src: None,
        }));

        let this_weak = Rc::downgrade(&this);
        {
            let mut this_mut = this.borrow_mut();
            this_mut.window.connect_delete_event(|_, _| {
                gtk::main_quit();
                Inhibit(false)
            });
            this_mut.window.set_titlebar(&this_mut.header_bar);

            let this_weak_clone = this_weak.clone();
            this_mut.play_pause_btn.connect_clicked(move |_| {
                let this = this_weak_clone.upgrade()
                    .expect("Main controller is no longer available for play/pause");
                this.borrow_mut().play_pause();
            });

            let this_weak = Rc::downgrade(&this);
            this_mut.self_weak = Some(this_weak);
        }

        let open_btn: Button = builder.get_object("open-btn").unwrap();
        let this_weak_clone = this_weak.clone();
        open_btn.connect_clicked(move |_| {
            let this = this_weak_clone.upgrade()
                .expect("Main controller is no longer available for select_media");
            this.borrow_mut().select_media();
        });

        this
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }

    pub fn play_pause(&mut self) {
        match self.ctx {
            Some(ref mut ctx) => match ctx.play_pause().unwrap() {
                gst::State::Playing => self.play_pause_btn.set_icon_name("media-playback-pause"),
                gst::State::Paused => self.play_pause_btn.set_icon_name("media-playback-start"),
                _ => (),
            },
            None => (),
        };
    }

    pub fn stop(&mut self) {
        if let Some(context) = self.ctx.as_mut() {
            context.stop();
            if let Some(source_id) = self.listener_src {
                // remove listerner in order to avoid conflict on borrowing of self
                glib::source_remove(source_id);
            }
            self.listener_src = None;
        }
        self.play_pause_btn.set_icon_name("media-playback-start");
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

                let context = self.ctx.as_ref()
                    .expect("Received InitDone, but context is not available");

                self.info_ctrl.new_media(context);
                self.video_ctrl.new_media(context);
                self.audio_ctrl.borrow_mut().new_media(context);

                self.header_bar.set_subtitle(Some(context.file_name.as_str()));
            },
            Eos => {
                println!("Received Eos");
                self.play_pause_btn.set_icon_name("media-playback-start");
                // TODO: seek to the begining
                keep_going = false;
            },
            FailedToOpenMedia => {
                eprintln!("ERROR: failed to open media");
                self.ctx = None;
                keep_going = false;
            },
            HaveAudioBuffer(audio_buffer) => {
                self.audio_ctrl.borrow_mut().have_buffer(audio_buffer);
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

        if result == ResponseType::Ok.into() {
            self.open_media(file_dlg.get_filename().unwrap());
        }
        else { () }

        file_dlg.close();
    }

    // TODO: change name `listener` as it is no longer just a listener
    // but rather a controller loop
    fn register_listener(&mut self,
        ui_rx: Receiver<ContextMessage>,
        timeout: u32,
        ui_tx_opt: Option<Sender<ContextMessage>>,
    )
    {
        let this_weak = self.self_weak.as_ref()
            .expect("Failed to get ref on MainController's weak Rc for register_listener")
            .clone();

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut keep_going = true;
            let mut msg_iter = ui_rx.try_iter();

            let this = this_weak.upgrade()
                .expect("Main controller is no longer available for ctx channel listener");
            let mut this_mut = this.borrow_mut();

            for msg in msg_iter.next() {
                keep_going = this_mut.process_message(msg, ui_tx_opt.as_ref());
                if !keep_going { break; }
            }

            let position = match this_mut.ctx {
                Some(ref ctx) => if ctx.state == gst::State::Playing {
                    let position = Timestamp::from_signed_nano(ctx.get_position());
                    this_mut.position_lbl.set_text(&format!("{}", position));
                    position
                }
                else {
                    Timestamp::new()
                },
                None => Timestamp::new(),
            };

            if position.nano > 0f64 {
                this_mut.audio_ctrl.borrow_mut().have_position(position.nano);
            }

            if !keep_going { println!("Exiting listener"); }
            glib::Continue(keep_going)
        }));
    }

    fn open_media(&mut self, filepath: PathBuf) {
        assert_eq!(self.listener_src, None);

        self.audio_ctrl.borrow_mut().clear();
        self.position_lbl.set_text("00:00.000");

        let (ctx_tx, ui_rx) = channel();
        let (ui_tx, ctx_rx) = channel();

        self.register_listener(ui_rx, 20, Some(ui_tx));

        match Context::open_media_path(filepath, ctx_tx, ctx_rx) {
            Ok(ctx) => self.ctx = Some(ctx),
            Err(error) => eprintln!("Error opening media: {}", error),
        };
    }
}
