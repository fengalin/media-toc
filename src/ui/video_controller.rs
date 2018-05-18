use gettextrs::gettext;
use glib;
use glib::{ObjectExt, ToValue};
use glib::signal::SignalHandlerId;
use gstreamer as gst;
use gstreamer::prelude::*;
use gtk;
use gtk::{BoxExt, ContainerExt, Inhibit, WidgetExt};

use std::cell::RefCell;
use std::rc::Rc;

use media::PlaybackContext;

use metadata::MediaInfo;

use super::MainController;

struct VideoOutput {
    sink: gst::Element,
    widget: gtk::Widget,
}

pub struct VideoController {
    video_output: Option<VideoOutput>,
    container: gtk::Box,
    cleaner_id: Option<SignalHandlerId>,
}

impl VideoController {
    pub fn new(builder: &gtk::Builder, disable_gl: bool) -> Self {
        let video_output = if !disable_gl {
                gst::ElementFactory::make("gtkglsink", "video_sink")
                    .and_then(|gtkglsink| {
                        let glsinkbin = gst::ElementFactory::make("glsinkbin", "video_sink_bin")
                            .expect("PlaybackContext: couldn't get `glsinkbin` from `gtkglsink`");
                        glsinkbin.set_property("sink", &gtkglsink.to_value())
                            .expect("VideoController: couldn't set `sink` for `glsinkbin`");

                        // Make sure the `glsink` is operational
                        let must_try_gl = match gst::ElementFactory::make("fakevideosink", None) {
                            Some(fake_src) => {
                                let glsinkbin = glsinkbin.clone();
                                let pipeline = gst::Pipeline::new("pipeline");
                                pipeline.add_many(&[&fake_src, &glsinkbin]).unwrap();
                                match fake_src.link(&glsinkbin) {
                                    Ok(()) => glsinkbin.sync_state_with_parent().is_ok(),
                                    Err(_) => false,
                                }
                            }
                            None => {
                                warn!("can't check wether `glsink` is operational beforehand. Install `gst-plugins-bad`.");
                                true
                            }
                        };

                        if must_try_gl {
                            debug!("Using gtkglsink");
                            Some(VideoOutput {
                                sink: glsinkbin,
                                widget: gtkglsink.get_property("widget")
                                    .expect(
                                        "VideoController: couldn't get `widget` from `gtkglsink`"
                                    )
                                    .get::<gtk::Widget>()
                                    .expect(
                                        "VideoController: unexpected type for `widget` in `gtkglsink`"
                                    ),
                            })
                        } else {
                            None
                        }
                    })
            } else {
                None
            }.or_else(|| {
                gst::ElementFactory::make("gtksink", "video_sink").map(|sink| {
                    debug!("Using gtksink");
                    VideoOutput {
                        sink: sink.clone(),
                        widget: sink.get_property("widget")
                            .expect("PlaybackContext: couldn't get `widget` from `gtksink`")
                            .get::<gtk::Widget>()
                            .expect(
                                "PlaybackContext: unexpected type for `widget` in `gtksink`"
                            ),
                    }
                })
            });

        let container: gtk::Box = builder.get_object("video-container").unwrap();
        match video_output {
            Some(ref video_output) => {
                container.pack_start(&video_output.widget, true, true, 0);
                container.reorder_child(&video_output.widget, 0);
            }
            None => {
                error!("{}", gettext("Couldn't find GStreamer GTK video sink."));
                let container = container.clone();
                gtk::idle_add(move || {
                    container.hide();
                    glib::Continue(false)
                });
            }
        };

        VideoController {
            video_output,
            container,
            cleaner_id: None,
        }
    }

    pub fn register_callbacks(&mut self, main_ctrl: &Rc<RefCell<MainController>>) {
        if self.video_output.is_some() {
            let main_ctrl_clone = Rc::clone(main_ctrl);
            self.container
                .connect_button_press_event(move |_, _event_button| {
                    main_ctrl_clone.borrow_mut().play_pause();
                    Inhibit(false)
                });
        }
    }

    pub fn get_video_sink(&self) -> Option<gst::Element> {
        self.video_output.as_ref()
            .map(|video_output| {
                video_output.sink.clone()
            })
    }

    fn get_video_widget(&self) -> Option<gtk::Widget> {
        self.video_output.as_ref()
            .map(|video_output| {
                video_output.widget.clone()
            })
    }

    pub fn cleanup(&mut self) {
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

    pub fn new_media(&mut self, context: &PlaybackContext) {
        let info = context.info.read().unwrap();
        self.streams_changed(&info);
    }

    pub fn streams_changed(&mut self, info: &MediaInfo) {
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
