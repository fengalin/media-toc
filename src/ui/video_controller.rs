use gtk;

use glib;
use glib::ObjectExt;
use glib::signal::SignalHandlerId;
use gtk::{BoxExt, ContainerExt, Inhibit, WidgetExt};

use std::rc::Rc;
use std::cell::RefCell;

use media::PlaybackContext;

use metadata::MediaInfo;

use super::MainController;

pub struct VideoController {
    is_available: bool,
    container: gtk::Box,
    cleaner_id: Option<SignalHandlerId>,
}

impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        VideoController {
            is_available: false,
            container: builder.get_object("video-container").unwrap(),
            cleaner_id: None,
        }
    }

    pub fn register_callbacks(&mut self, main_ctrl: &Rc<RefCell<MainController>>) {
        match PlaybackContext::get_video_widget() {
            Some(video_widget) => {
                self.container.pack_start(&video_widget, true, true, 0);
                self.container.reorder_child(&video_widget, 0);

                let main_ctrl_clone = Rc::clone(main_ctrl);
                self.container
                    .connect_button_press_event(move |_, _event_button| {
                        main_ctrl_clone.borrow_mut().play_pause();
                        Inhibit(false)
                    });

                self.is_available = true;
            }
            None => {
                let container = self.container.clone();
                gtk::idle_add(move || {
                    container.hide();
                    glib::Continue(false)
                });
                self.is_available = false;
            }
        }
    }

    pub fn cleanup(&mut self) {
        if self.is_available && self.cleaner_id.is_none() {
            let video_widget = &self.container.get_children()[0];
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

    pub fn new_media(&mut self, context: &PlaybackContext) {
        let info = context.info.lock().unwrap();
        self.streams_changed(&info);
    }

    pub fn streams_changed(&mut self, info: &MediaInfo) {
        if self.is_available {
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
