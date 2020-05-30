use glib::{signal::SignalHandlerId, ObjectExt, ToValue};
use gstreamer as gst;
use gtk::prelude::*;
use log::debug;

use application::{CommandLineArguments, CONFIG};
use metadata::MediaInfo;

use super::UIController;

pub struct VideoOutput {
    sink: gst::Element,
    pub(super) widget: gtk::Widget,
}

pub struct VideoController {
    pub(super) video_output: Option<VideoOutput>,
    pub(super) container: gtk::Box,
    cleaner_id: Option<SignalHandlerId>,
}

impl UIController for VideoController {
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
    pub fn new(builder: &gtk::Builder, args: &CommandLineArguments) -> Self {
        let container: gtk::Box = builder.get_object("video-container").unwrap();

        let video_output = if !args.disable_gl && !CONFIG.read().unwrap().media.is_gl_disabled {
            gst::ElementFactory::make("gtkglsink", Some("gtkglsink"))
                .map(|gtkglsink| {
                    let glsinkbin = gst::ElementFactory::make("glsinkbin", Some("video_sink"))
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
                            .expect("VideoController: unexpected type for `widget` in `gtkglsink`")
                            .expect("VideoController: `widget` not found in `gtkglsink`"),
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
                        widget: sink
                            .get_property("widget")
                            .expect("VideoController: couldn't get `widget` from `gtksink`")
                            .get::<gtk::Widget>()
                            .expect("VideoController: unexpected type for `widget` in `gtksink`")
                            .expect("VideoController: `widget` not found in `gtksink`"),
                    }
                })
                .ok()
        });

        if let Some(video_output) = video_output.as_ref() {
            container.pack_start(&video_output.widget, true, true, 0);
            container.reorder_child(&video_output.widget, 0);
            video_output.widget.show();
        };

        let mut video_ctrl = VideoController {
            video_output,
            container,
            cleaner_id: None,
        };

        video_ctrl.cleanup();

        video_ctrl
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
