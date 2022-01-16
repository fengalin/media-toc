use gtk::{glib::signal::SignalHandlerId, prelude::*};
use log::debug;

use ::application::{CommandLineArguments, CONFIG};
use ::metadata::MediaInfo;

use crate::prelude::*;

pub struct VideoOutput {
    sink: gst::Element,
    pub(super) widget: gtk::Widget,
}

pub struct Controller {
    pub(super) video_output: Option<VideoOutput>,
    pub(super) container: gtk::Box,
    cleaner_id: Option<SignalHandlerId>,
}

impl UIController for Controller {
    fn cleanup(&mut self) {
        if let Some(video_widget) = self.video_widget() {
            if self.cleaner_id.is_none() {
                self.cleaner_id = Some(video_widget.connect_draw(|widget, cr| {
                    let allocation = widget.allocation();
                    cr.set_source_rgb(0f64, 0f64, 0f64);
                    cr.rectangle(
                        0f64,
                        0f64,
                        f64::from(allocation.width()),
                        f64::from(allocation.height()),
                    );
                    cr.fill().unwrap();

                    Inhibit(true)
                }));
                video_widget.queue_draw();
            }
        }
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        if self.video_output.is_some() {
            if let Some(cleaner_id) = self.cleaner_id.take() {
                self.container.children()[0].disconnect(cleaner_id);
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

impl Controller {
    pub fn new(builder: &gtk::Builder, args: &CommandLineArguments) -> Self {
        let container: gtk::Box = builder.object("video-container").unwrap();

        let video_output = if !args.disable_gl && !CONFIG.read().unwrap().media.is_gl_disabled {
            gst::ElementFactory::make("gtkglsink", Some("gtkglsink"))
                .map(|gtkglsink| {
                    let glsinkbin = gst::ElementFactory::make("glsinkbin", Some("video_sink"))
                        .expect("PlaybackPipeline: couldn't get `glsinkbin` from `gtkglsink`");
                    glsinkbin.set_property("sink", &gtkglsink);

                    debug!("Using gtkglsink");
                    VideoOutput {
                        sink: glsinkbin,
                        widget: gtkglsink.property::<gtk::Widget>("widget"),
                    }
                })
                .ok()
        } else {
            None
        }
        .or_else(|| {
            gst::ElementFactory::make("gtksink", Some("video_sink"))
                .map(|sink| {
                    debug!("Using gtksink");
                    VideoOutput {
                        sink: sink.clone(),
                        widget: sink.property::<gtk::Widget>("widget"),
                    }
                })
                .ok()
        });

        if let Some(video_output) = video_output.as_ref() {
            container.pack_start(&video_output.widget, true, true, 0);
            container.reorder_child(&video_output.widget, 0);
            video_output.widget.show();
        };

        let mut video_ctrl = Controller {
            video_output,
            container,
            cleaner_id: None,
        };

        video_ctrl.cleanup();

        video_ctrl
    }

    pub fn video_sink(&self) -> Option<gst::Element> {
        self.video_output
            .as_ref()
            .map(|video_output| video_output.sink.clone())
    }

    fn video_widget(&self) -> Option<gtk::Widget> {
        self.video_output
            .as_ref()
            .map(|video_output| video_output.widget.clone())
    }
}
