use gdk;
use gettextrs::gettext;
use glib;
use glib::{signal::SignalHandlerId, ObjectExt, ToValue};
use gstreamer as gst;
use gtk;
use gtk::prelude::*;
use log::{debug, error};

use std::{cell::RefCell, rc::Rc};

use crate::{application::CONFIG, media::PlaybackPipeline, metadata::MediaInfo};

use super::{MainController, UIController};

struct VideoOutput {
    sink: gst::Element,
    widget: gtk::Widget,
}

pub struct VideoController {
    disable_gl: bool,
    video_output: Option<VideoOutput>,
    container: gtk::Box,
    cleaner_id: Option<SignalHandlerId>,
}

impl UIController for VideoController {
    // TODO: use this instead of setup_ when all controllers are not longer on the heap
    fn setup(
        _this_rc: &Rc<RefCell<Self>>,
        _gtk_app: &gtk::Application,
        _main_ctrl: &Rc<RefCell<MainController>>,
    ) {
    }

    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        let info = pipeline.info.read().unwrap();
        self.streams_changed(&info);
    }

    fn cleanup(&mut self) {
        if let Some(video_widget) = self.get_video_widget() {
            if self.cleaner_id.is_none() {
                self.cleaner_id = Some(video_widget.connect_draw(|widget, cr| {
                    let allocation = widget.get_allocation();
                    cr.set_source_rgb(0f64, 0f64, 0f64);
                    cr.rectangle(
                        0f64,
                        0f64,
                        f64::from(allocation.width),
                        f64::from(allocation.height),
                    );
                    cr.fill();

                    Inhibit(true)
                }));
                video_widget.queue_draw();
            }
        }
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        if self.video_output.is_some() {
            if let Some(cleaner_id) = self.cleaner_id.take() {
                self.container.get_children()[0].disconnect(cleaner_id);
            }

            if info.streams.is_video_selected() {
                debug!("streams_changed video selected");
                self.container.show();
            } else {
                debug!("streams_changed video not selected");
                self.container.hide();
            }
        }
    }
}

impl VideoController {
    pub fn new(builder: &gtk::Builder, disable_gl: bool) -> Self {
        VideoController {
            disable_gl,
            video_output: None,
            container: builder.get_object("video-container").unwrap(),
            cleaner_id: None,
        }
    }

    // TODO: rename to setup when all controllers are not longer on the heap
    pub fn setup_(&mut self, main_ctrl: &Rc<RefCell<MainController>>) {
        let video_output = if !self.disable_gl && !CONFIG.read().unwrap().media.is_gl_disabled {
            gst::ElementFactory::make("gtkglsink", "gtkglsink").map(|gtkglsink| {
                let glsinkbin = gst::ElementFactory::make("glsinkbin", "video_sink")
                    .expect("PlaybackPipeline: couldn't get `glsinkbin` from `gtkglsink`");
                glsinkbin
                    .set_property("sink", &gtkglsink.to_value())
                    .expect("VideoController: couldn't set `sink` for `glsinkbin`");

                debug!("Using gtkglsink");
                VideoOutput {
                    sink: glsinkbin,
                    widget: gtkglsink
                        .get_property("widget")
                        .expect("VideoController: couldn't get `widget` from `gtkglsink`")
                        .get::<gtk::Widget>()
                        .expect("VideoController: unexpected type for `widget` in `gtkglsink`"),
                }
            })
        } else {
            None
        }
        .or_else(|| {
            gst::ElementFactory::make("gtksink", "video_sink").map(|sink| {
                debug!("Using gtksink");
                VideoOutput {
                    sink: sink.clone(),
                    widget: sink
                        .get_property("widget")
                        .expect("PlaybackPipeline: couldn't get `widget` from `gtksink`")
                        .get::<gtk::Widget>()
                        .expect("PlaybackPipeline: unexpected type for `widget` in `gtksink`"),
                }
            })
        });

        match video_output {
            Some(ref video_output) => {
                // discard GStreamer defined navigation events on widget
                video_output
                    .widget
                    .set_events(gdk::EventMask::BUTTON_PRESS_MASK);

                self.container
                    .pack_start(&video_output.widget, true, true, 0);
                self.container.reorder_child(&video_output.widget, 0);
                video_output.widget.show();
                self.cleanup();
                let main_ctrl_clone = Rc::clone(main_ctrl);
                self.container
                    .connect_button_press_event(move |_, _event_button| {
                        main_ctrl_clone.borrow_mut().play_pause();
                        Inhibit(true)
                    });
            }
            None => {
                error!("{}", gettext("Couldn't find GStreamer GTK video sink."));
                let container = self.container.clone();
                gtk::idle_add(move || {
                    container.hide();
                    glib::Continue(false)
                });
            }
        };

        self.video_output = video_output;
    }

    pub fn get_video_sink(&self) -> Option<gst::Element> {
        self.video_output
            .as_ref()
            .map(|video_output| video_output.sink.clone())
    }

    fn get_video_widget(&self) -> Option<gtk::Widget> {
        self.video_output
            .as_ref()
            .map(|video_output| video_output.widget.clone())
    }
}
