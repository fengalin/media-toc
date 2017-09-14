extern crate gtk;
extern crate glib;
extern crate gstreamer as gst;

#[cfg(any(feature = "profiling-listener", feature = "profiling-tracker"))]
use chrono::Utc;

use std::rc::Rc;
use std::cell::RefCell;

use std::path::PathBuf;

use std::sync::mpsc::{channel, Receiver};

use gtk::prelude::*;

use ::media::{Context, ContextMessage, Timestamp};
use ::media::ContextMessage::*;

use super::{AudioController, DoubleWaveformBuffer, InfoController, VideoController};

pub struct MainController {
    window: gtk::ApplicationWindow,
    header_bar: gtk::HeaderBar,
    play_pause_btn: gtk::ToolButton,
    position_lbl: gtk::Label,
    timeline_scale: gtk::Scale,
    info_ctrl: InfoController,
    video_ctrl: VideoController,
    audio_ctrl: Rc<RefCell<AudioController>>,
    audio_drawingarea: gtk::DrawingArea,

    context: Option<Context>,

    this_opt: Option<Rc<RefCell<MainController>>>,
    keep_going: bool,
    listener_src: Option<glib::SourceId>,
    tracker_src: Option<glib::SourceId>,
}

impl MainController {
    pub fn new(builder: gtk::Builder) -> Rc<RefCell<Self>> {
        let audio_controller = AudioController::new(&builder);
        let audio_drawingarea = audio_controller.borrow().drawingarea.clone();

        let this = Rc::new(RefCell::new(MainController {
            window: builder.get_object("application-window").unwrap(),
            header_bar: builder.get_object("header-bar").unwrap(),
            play_pause_btn: builder.get_object("play_pause-toolbutton").unwrap(),
            position_lbl: builder.get_object("position-lbl").unwrap(),
            timeline_scale: builder.get_object("timeline-scale").unwrap(),
            info_ctrl: InfoController::new(&builder),
            video_ctrl: VideoController::new(&builder),
            audio_ctrl: audio_controller,
            audio_drawingarea: audio_drawingarea,

            context: None,

            this_opt: None,
            keep_going: true,
            listener_src: None,
            tracker_src: None,
        }));

        {
            let mut this_mut = this.borrow_mut();

            let this_rc = this.clone();
            this_mut.this_opt = Some(this_rc);

            this_mut.window.connect_delete_event(|_, _| {
                gtk::main_quit();
                Inhibit(false)
            });
            this_mut.window.set_titlebar(&this_mut.header_bar);

            let this_rc = this.clone();
            this_mut.play_pause_btn.connect_clicked(move |_| {
                this_rc.borrow_mut().play_pause();
            });
        }

        let open_btn: gtk::Button = builder.get_object("open-btn").unwrap();
        let this_rc = this.clone();
        open_btn.connect_clicked(move |_| {
            this_rc.borrow_mut().select_media();
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
                self.register_tracker(17); // 60 Hz
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

        let file_dlg = gtk::FileChooserDialog::new(
            Some("Open a media file"),
            Some(&self.window),
            gtk::FileChooserAction::Open,
        );
        // Note: couldn't find equivalents for STOCK_OK
        file_dlg.add_button("Open", gtk::ResponseType::Ok.into());

        if file_dlg.run() == gtk::ResponseType::Ok.into() {
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
    ) {
        let this_rc = self.this_opt.as_ref()
            .unwrap()
            .clone();

        self.listener_src = Some(gtk::timeout_add(timeout, move || {
            #[cfg(feature = "profiling-listener")]
            let start = Utc::now();

            let mut message_iter = ui_rx.try_iter();

            let mut keep_going = true;

            #[cfg(feature = "profiling-listener")]
            let before_loop = Utc::now();

            for message in message_iter.next() {
                match message {
                    AsyncDone => {
                        println!("Received AsyncDone");
                    },
                    InitDone => {
                        println!("Received InitDone");

                        let mut this_mut = this_rc.borrow_mut();

                        let context = this_mut.context.take()
                            .expect("... but no context available");

                        this_mut.header_bar.set_subtitle(
                            Some(context.file_name.as_str())
                        );

                        this_mut.info_ctrl.new_media(&context);
                        this_mut.video_ctrl.new_media(&context);
                        this_mut.audio_ctrl.borrow_mut().new_media(&context);

                        this_mut.context = Some(context);
                    },
                    Eos => {
                        println!("Received Eos");

                        let mut this_mut = this_rc.borrow_mut();

                        this_mut.audio_drawingarea.queue_draw();

                        this_mut.play_pause_btn.set_icon_name("media-playback-start");

                        this_mut.keep_going = false;
                        keep_going = false;
                    },
                    FailedToOpenMedia => {
                        eprintln!("ERROR: failed to open media");

                        let mut this_mut = this_rc.borrow_mut();

                        this_mut.context = None;
                        this_mut.keep_going = false;
                        keep_going = false;
                    },
                };

                if !keep_going { break; }
            }

            #[cfg(feature = "profiling-listener")]
            let end = Utc::now();

            #[cfg(feature = "profiling-listener")]
            println!("listener,{},{},{}",
                start.time().format("%H:%M:%S%.6f"),
                before_loop.time().format("%H:%M:%S%.6f"),
                end.time().format("%H:%M:%S%.6f"),
            );

            if !keep_going {
                let mut this_mut = this_rc.borrow_mut();
                this_mut.listener_src = None;
                this_mut.tracker_src = None;
            }

            glib::Continue(keep_going)
        }));
    }

    fn remove_tracker(&mut self) {
        if let Some(source_id) = self.tracker_src.take() {
            glib::source_remove(source_id);
        }
    }

    fn register_tracker(&mut self, timeout: u32) {
        let this_rc = self.this_opt.as_ref()
            .unwrap()
            .clone();

        self.tracker_src = Some(gtk::timeout_add(timeout, move || {
            #[cfg(feature = "profiling-tracker")]
            let start = Utc::now();

            let mut this_mut = this_rc.borrow_mut();

            #[cfg(feature = "profiling-tracker")]
            let before_position = Utc::now();

            let position = this_mut.context.as_mut()
                .expect("No context in tracker")
                .get_position();

            #[cfg(feature = "profiling-tracker")]
            let before_tic = Utc::now();

            this_mut.timeline_scale.set_value(position as f64);
            this_mut.position_lbl.set_text(&Timestamp::format(position));
            this_mut.audio_drawingarea.queue_draw();

            #[cfg(feature = "profiling-tracker")]
            let end = Utc::now();

            #[cfg(feature = "profiling-tracker")]
            println!("tracker,{},{},{},{},{}",
                start.time().format("%H:%M:%S%.6f"),
                before_position.time().format("%H:%M:%S%.6f"),
                before_tic.time().format("%H:%M:%S%.6f"),
                end.time().format("%H:%M:%S%.6f"),
                position,
            );

            glib::Continue(this_mut.keep_going)
        }));
    }

    fn open_media(&mut self, filepath: PathBuf) {
        assert_eq!(self.listener_src, None);

        self.timeline_scale.set_value(0f64);
        self.position_lbl.set_text("00:00.000");

        let (ctx_tx, ui_rx) = channel();

        self.keep_going = true;
        self.register_listener(500, ui_rx);

        match Context::new(
            filepath,
            5_000_000_000,
            DoubleWaveformBuffer::new(),
            self.video_ctrl.video_box.clone(),
            ctx_tx
        ) {
            Ok(context) => {
                self.context = Some(context);
            },
            Err(error) => eprintln!("Error opening media: {}", error),
        };
    }
}
