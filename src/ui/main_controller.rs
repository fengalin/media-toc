extern crate gtk;
extern crate glib;
extern crate gstreamer as gst;

use std::rc::{Rc, Weak};
use std::cell::RefCell;

use std::path::PathBuf;

use std::sync::mpsc::{channel, Receiver};

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

    context: Option<Context>,
    duration: u64,
    last_position: u64,

    self_weak: Option<Weak<RefCell<MainController>>>,
    keep_going: bool,
    listener_src: Option<glib::SourceId>,
    tracker_src: Option<glib::SourceId>,
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

            context: None,
            duration: 0,
            last_position: 0,

            self_weak: None,
            keep_going: true,
            listener_src: None,
            tracker_src: None,
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
                if let Some(this_ref) = this_weak_clone.upgrade() {
                    this_ref.borrow_mut().play_pause();
                };
            });

            let this_weak = Rc::downgrade(&this);
            this_mut.self_weak = Some(this_weak);
        }

        let open_btn: Button = builder.get_object("open-btn").unwrap();
        let this_weak_clone = this_weak.clone();
        open_btn.connect_clicked(move |_| {
            if let Some(this_ref) = this_weak_clone.upgrade() {
                this_ref.borrow_mut().select_media();
            }
        });

        this
    }

    pub fn show_all(&self) {
        self.window.show_all();
    }

    pub fn play_pause(&mut self) {
        let context =
            match self.context.take() {
                Some(context) => context,
                None => return,
            };

        match context.get_state() {
            gst::State::Paused => {
                self.register_tracker(25); // 40 Hz
                self.play_pause_btn.set_icon_name("media-playback-pause");
                context.play().unwrap();
            }
            gst::State::Playing => {
                context.pause().unwrap();
                self.play_pause_btn.set_icon_name("media-playback-start");
                self.remove_tracker();
            },
            state => println!("Can't play/pause in state {:?}", state),
        };

        self.context = Some(context);
    }

    pub fn stop(&mut self) {
        if let Some(context) = self.context.as_mut() {
            context.stop();
        };

        // remove callbacks in order to avoid conflict on borrowing of self
        self.remove_listener();
        self.remove_tracker();

        self.audio_ctrl.borrow_mut().cleanup();

        self.play_pause_btn.set_icon_name("media-playback-start");
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

        if file_dlg.run() == ResponseType::Ok.into() {
            self.open_media(file_dlg.get_filename().unwrap());
        }

        file_dlg.close();
    }

    fn remove_listener(&mut self) {
        if let Some(source_id) = self.listener_src.take() {
            glib::source_remove(source_id);
        }
    }

    fn register_listener(&mut self,
        timeout: u32,
        ui_rx: Receiver<ContextMessage>,
    )
    {
        let this_weak = self.self_weak.as_ref()
            .unwrap()
            .clone();

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            let mut message_iter = ui_rx.try_iter();

            let mut keep_going = true;
            for message in message_iter.next() {
                match message {
                    AsyncDone => {
                        println!("Received AsyncDone");
                    },
                    InitDone => {
                        println!("Received InitDone");

                        let this_rc = this_weak.upgrade().unwrap();
                        let mut this_mut = this_rc.borrow_mut();

                        let context = this_mut.context.take()
                            .expect("... but no context available");

                        let duration = context.get_duration();
                        if duration.is_negative() {
                            panic!("Negative duration");
                        }
                        this_mut.duration = duration as u64;

                        this_mut.info_ctrl.new_media(&context);
                        this_mut.video_ctrl.new_media(&context);
                        this_mut.audio_ctrl.borrow_mut().new_media(&context);

                        this_mut.header_bar.set_subtitle(
                            Some(context.file_name.as_str())
                        );

                        this_mut.context = Some(context);
                    },
                    Eos => {
                        println!("Received Eos");
                        let this_rc = this_weak.upgrade().unwrap();
                        let mut this_mut = this_rc.borrow_mut();

                        this_mut.remove_tracker();
                        let duration = this_mut.duration;
                        this_mut.tic(duration);

                        // TODO: exit tracker
                        this_mut.play_pause_btn.set_icon_name("media-playback-start");
                    },
                    FailedToOpenMedia => {
                        eprintln!("ERROR: failed to open media");
                        let this_rc = this_weak.upgrade().unwrap();
                        let mut this_mut = this_rc.borrow_mut();

                        this_mut.context = None;
                        this_mut.keep_going = false;
                        keep_going = false;
                    },
                };

                if !keep_going { break; }
            }

            glib::Continue(keep_going)
        }));
    }

    fn remove_tracker(&mut self) {
        if let Some(source_id) = self.tracker_src.take() {
            glib::source_remove(source_id);
        }
    }

    fn tic(&mut self, position: u64) {
        self.position_lbl.set_text(
            &format!("{}", Timestamp::from_nano(position as i64))
        );
        self.last_position = position;

        self.audio_ctrl.borrow_mut().tic(position);
    }

    fn register_tracker(&mut self, timeout: u32) {
        let this_weak = self.self_weak.as_ref()
            .unwrap()
            .clone();

        self.tracker_src = Some(gtk::timeout_add(timeout, move || {
            let this = this_weak.upgrade().unwrap();
            let mut this_mut = this.borrow_mut();

            if this_mut.keep_going {
                let position = this_mut.context.as_mut()
                    .expect("No context in tracker")
                    .get_position();
                if this_mut.last_position != position {
                    if position <= this_mut.duration {
                        this_mut.tic(position);
                    }
                }
            } else {
                this_mut.tracker_src = None;
                println!("Exiting tracker");
            }

            glib::Continue(this_mut.keep_going)
        }));
    }

    fn open_media(&mut self, filepath: PathBuf) {
        assert_eq!(self.listener_src, None);

        self.position_lbl.set_text("00:00.000");

        let (ctx_tx, ui_rx) = channel();

        self.keep_going = true;
        self.register_listener(500, ui_rx);

        match Context::open_media_path(
                filepath,
                10_000_000_000,
                self.video_ctrl.video_box.clone(),
                ctx_tx
            )
            {
            Ok(context) => {
                self.context = Some(context);
                self.last_position = 0;
            },
            Err(error) => eprintln!("Error opening media: {}", error),
        };
    }
}
