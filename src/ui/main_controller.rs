extern crate gtk;
extern crate glib;
extern crate gstreamer as gst;

/*extern crate chrono;
use chrono::Utc;*/

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

    context: Option<Rc<RefCell<Context>>>,
    last_position: u64,

    /*min_pos_query: u64,
    max_pos_query: u64,
    sum_pos_query: u64,
    nb_pos_query: u64,*/

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

            last_position: 0,
            /*min_pos_query: 0,
            max_pos_query: 0,
            sum_pos_query: 0,
            nb_pos_query: 0,*/

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
        let state =
            if let Some(context_rc) = self.context.as_ref() {
                let context = context_rc.borrow();
                match context.get_state() {
                    gst::State::Paused => {
                        context.play().unwrap();
                        gst::State::Paused
                    }
                    gst::State::Playing => {
                        context.pause().unwrap();
                        gst::State::Playing
                    },
                    state => {
                        println!("Can't play/pause in state {:?}", state);
                        return;
                    },
                }
            } else {
                return;
            };

        match state {
            gst::State::Paused => {
                self.register_tracker(16); // 60 Hz
                self.play_pause_btn.set_icon_name("media-playback-pause");
            }
            gst::State::Playing => {
                self.play_pause_btn.set_icon_name("media-playback-start");
                self.remove_tracker();
            },
            state => println!("Can't play/pause in state {:?}", state),
        };
    }

    pub fn stop(&mut self) {
        if let Some(context_rc) = self.context.as_ref() {
            context_rc.borrow().stop();
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
        if let Some(source_id) = self.listener_src {
            glib::source_remove(source_id);
        }
        self.listener_src = None;
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

            let this_rc = this_weak.upgrade().unwrap();
            let mut this_mut = this_rc.borrow_mut();

            for message in message_iter.next() {
                match message {
                    AsyncDone => {
                        println!("Received AsyncDone");
                    },
                    InitDone => {
                        println!("Received InitDone");

                        let context_rc = this_mut.context.as_ref()
                            .expect("... but no context available")
                            .clone();

                        this_mut.info_ctrl.new_media(&context_rc);
                        this_mut.video_ctrl.new_media(&context_rc);
                        this_mut.audio_ctrl.borrow_mut().new_media(&context_rc);

                        this_mut.header_bar.set_subtitle(
                            Some(context_rc.borrow().file_name.as_str())
                        );
                    },
                    Eos => {
                        println!("Received Eos");
                        this_mut.play_pause_btn.set_icon_name("media-playback-start");
                    },
                    FailedToOpenMedia => {
                        eprintln!("ERROR: failed to open media");
                        this_mut.context = None;
                        this_mut.keep_going = false;
                    },
                };

                if !this_mut.keep_going { break; }
            }

            if !this_mut.keep_going {
                this_mut.listener_src = None;
                println!("Exiting listener");
            }

            glib::Continue(this_mut.keep_going)
        }));
    }

    fn remove_tracker(&mut self) {
        if let Some(source_id) = self.tracker_src {
            glib::source_remove(source_id);
        }
        self.tracker_src = None;
    }

    fn register_tracker(&mut self, timeout: u32) {
        let this_weak = self.self_weak.as_ref()
            .unwrap()
            .clone();

        self.tracker_src = Some(gtk::timeout_add(timeout, move || {
            let this = this_weak.upgrade().unwrap();
            let mut this_mut = this.borrow_mut();

            if this_mut.keep_going {
                let context_rc = this_mut.context.as_ref()
                    .expect("Tracking... but no context available")
                    .clone();

                let mut context = context_rc.borrow_mut();

                //let before = Utc::now();
                let position = context.get_position();
                /*let after = Utc::now();
                let delta = after.signed_duration_since(before)
                    .num_nanoseconds()
                    .unwrap() as u64;

                this_mut.max_pos_query = this_mut.max_pos_query.max(delta);
                this_mut.sum_pos_query += delta;
                this_mut.nb_pos_query += 1;
                println!("{} - {} - {}", this_mut.max_pos_query, this_mut.sum_pos_query / this_mut.nb_pos_query, delta);

                if this_mut.nb_pos_query > 10 * 60 {
                    this_mut.max_pos_query = 0;
                    this_mut.sum_pos_query = 0;
                    this_mut.nb_pos_query = 0;
                }*/

                if this_mut.last_position != position {
                    this_mut.position_lbl.set_text(
                        &format!("{}", Timestamp::from_nano(position as i64))
                    );
                    this_mut.last_position = position;

                    this_mut.audio_ctrl.borrow_mut().tic(position);
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
                self.context = Some(Rc::new(RefCell::new(context)));
                self.last_position = 0;
            },
            Err(error) => eprintln!("Error opening media: {}", error),
        };
    }
}
