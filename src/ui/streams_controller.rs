extern crate gstreamer as gst;

extern crate gtk;
use gtk::prelude::*;

use std::rc::{Rc, Weak};
use std::cell::RefCell;

use media::PlaybackContext;

use super::MainController;

const ALIGN_LEFT: f32 = 0f32;
const ALIGN_CENTER: f32 = 0.5f32;
const ALIGN_RIGHT: f32 = 1f32;

const STREAM_ID_COL: u32 = 0;
const STREAM_ID_DISPLAY_COL: u32 = 1;
const LANGUAGE_COL: u32 = 2;
const CODEC_COL: u32 = 3;
const COMMENT_COL: u32 = 4;

const VIDEO_WIDTH_COL: u32 = 5;
const VIDEO_HEIGHT_COL: u32 = 6;

const AUDIO_RATE_COL: u32 = 5;
const AUDIO_CHANNELS_COL: u32 = 6;

const TEXT_FORMAT_COL: u32 = 5;

pub struct StreamsController {
    streams_button: gtk::ToggleButton,
    display_streams_stack: gtk::Stack,

    video_treeview: gtk::TreeView,
    video_store: gtk::ListStore,
    audio_treeview: gtk::TreeView,
    audio_store: gtk::ListStore,
    text_treeview: gtk::TreeView,
    text_store: gtk::ListStore,

    main_ctrl: Option<Weak<RefCell<MainController>>>,
}

impl StreamsController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this_rc = Rc::new(RefCell::new(StreamsController {
            streams_button: builder.get_object("streams-toggle").unwrap(),
            display_streams_stack: builder.get_object("display_streams-stack").unwrap(),

            video_treeview: builder.get_object("video_streams-treeview").unwrap(),
            video_store: builder.get_object("video_streams-liststore").unwrap(),
            audio_treeview: builder.get_object("audio_streams-treeview").unwrap(),
            audio_store: builder.get_object("audio_streams-liststore").unwrap(),
            text_treeview: builder.get_object("text_streams-treeview").unwrap(),
            text_store: builder.get_object("text_streams-liststore").unwrap(),

            main_ctrl: None,
        }));

        {
            let mut this = this_rc.borrow_mut();
            this.cleanup();
            this.init_treeviews();
        }

        this_rc
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();

        this.main_ctrl = Some(Rc::downgrade(main_ctrl));

        // streams button
        let this_clone = Rc::clone(this_rc);
        this.streams_button.connect_clicked(move |button| {
            let page_name = if button.get_active() {
                "streams".into()
            } else {
                "display".into()
            };
            this_clone.borrow_mut().display_streams_stack.set_visible_child_name(page_name);
        });
    }

    pub fn new_media(&mut self, context: &PlaybackContext) {
        self.streams_button.set_sensitive(true);

        let info = context
            .info
            .lock()
            .expect("StreamsController::new_media: failed to lock media info");

        // Video streams
        for (stream_id, &(ref caps, ref tags)) in &info.video_streams {
            let iter = self.add_stream(&self.video_store, &stream_id, caps, tags);
            let caps_structure = caps.get_structure(0).unwrap();
            if let Some(width) = caps_structure.get::<i32>("width") {
                self.video_store.set_value(&iter, VIDEO_WIDTH_COL, &gtk::Value::from(&width));
            }
            if let Some(height) = caps_structure.get::<i32>("height") {
                self.video_store.set_value(&iter, VIDEO_HEIGHT_COL, &gtk::Value::from(&height));
            }
        }

        match self.video_store.get_iter_first() {
            Some(ref iter) => self.video_treeview.get_selection().select_iter(iter),
            None => (),
        }

        // Audio streams
        for (stream_id, &(ref caps, ref tags)) in &info.audio_streams {
            let iter = self.add_stream(&self.audio_store, &stream_id, caps, tags);
            let caps_structure = caps.get_structure(0).unwrap();
            if let Some(rate) = caps_structure.get::<i32>("rate") {
                self.audio_store.set_value(&iter, AUDIO_RATE_COL, &gtk::Value::from(&rate));
            }
            if let Some(channels) = caps_structure.get::<i32>("channels") {
                self.audio_store.set_value(&iter, AUDIO_CHANNELS_COL, &gtk::Value::from(&channels));
            }
        }

        match self.audio_store.get_iter_first() {
            Some(ref iter) => self.audio_treeview.get_selection().select_iter(iter),
            None => (),
        }

        // Text streams
        for (stream_id, &(ref caps, ref tags)) in &info.text_streams {
            let iter = self.add_stream(&self.text_store, &stream_id, caps, tags);
            let caps_structure = caps.get_structure(0).unwrap();
            if let Some(format) = caps_structure.get::<&str>("format") {
                self.text_store.set_value(&iter, TEXT_FORMAT_COL, &gtk::Value::from(&format));
            }
        }

        match self.text_store.get_iter_first() {
            Some(ref iter) => self.text_treeview.get_selection().select_iter(iter),
            None => (),
        }
    }

    fn add_stream(
        &self,
        store: &gtk::ListStore,
        stream_id: &str,
        caps: &gst::Caps,
        tags: &Option<gst::TagList>,
    ) -> gtk::TreeIter {
        let id_parts: Vec<&str> = stream_id.split('/').collect();
        let stream_id_display = if id_parts.len() == 2 {
            id_parts[1]
        } else {
            "unknown"
        };

        let iter = store.insert_with_values(
            None,
            &[STREAM_ID_COL, STREAM_ID_DISPLAY_COL],
            &[&stream_id, &stream_id_display],
        );

        let codec = match tags.as_ref() {
            Some(tags) => {
                let language = match tags.get_index::<gst::tags::LanguageName>(0).as_ref() {
                    Some(language) => language.get().unwrap(),
                    None => match tags.get_index::<gst::tags::LanguageCode>(0).as_ref() {
                        Some(language) => language.get().unwrap(),
                        None => "-",
                    }
                };
                store.set_value(&iter, LANGUAGE_COL, &gtk::Value::from(language));

                if let Some(comment) = tags.get_index::<gst::tags::Comment>(0).as_ref() {
                    store.set_value(&iter, COMMENT_COL, &gtk::Value::from(comment.get().unwrap()));
                }

                match tags.get_index::<gst::tags::VideoCodec>(0).as_ref() {
                    Some(codec) => codec.get(),
                    None => match tags.get_index::<gst::tags::AudioCodec>(0).as_ref() {
                        Some(codec) => codec.get(),
                        None => match tags.get_index::<gst::tags::SubtitleCodec>(0).as_ref() {
                            Some(codec) => codec.get(),
                            None => match tags.get_index::<gst::tags::Codec>(0).as_ref() {
                                Some(codec) => codec.get(),
                                None => None,
                            }
                        }
                    }
                }
            }
            None => None,
        };

        let codec = match codec {
            Some(codec) => codec,
            None => {
                // codec in caps in the form "streamtype/x-codec"
                let codec = caps.get_structure(0).unwrap().get_name();
                let id_parts: Vec<&str> = codec.split('/').collect();
                if id_parts.len() == 2 {
                    if id_parts[1].starts_with("x-") {
                        &id_parts[1][2..]
                    } else {
                        id_parts[1]
                    }
                } else {
                    codec
                }
            },
        };
        store.set_value(&iter, CODEC_COL, &gtk::Value::from(&codec));

        iter
    }

    pub fn cleanup(&mut self) {
        self.streams_button.set_sensitive(false);
        self.video_store.clear();
        self.audio_store.clear();
        self.text_store.clear();
    }

    fn init_treeviews(&self) {
        self.video_treeview.set_model(Some(&self.video_store));
        self.add_column(
            &self.video_treeview,
            "Stream id",
            ALIGN_LEFT,
            STREAM_ID_DISPLAY_COL,
            Some(200),
        );
        self.add_column(&self.video_treeview, "Lang.", ALIGN_CENTER, LANGUAGE_COL, None);
        self.add_column(&self.video_treeview, "Codec", ALIGN_LEFT, CODEC_COL, None);
        self.add_column(&self.video_treeview, "Width", ALIGN_RIGHT, VIDEO_WIDTH_COL, None);
        self.add_column(&self.video_treeview, "Height", ALIGN_RIGHT, VIDEO_HEIGHT_COL, None);
        self.add_column(&self.video_treeview, "Comment", ALIGN_LEFT, COMMENT_COL, None);

        self.audio_treeview.set_model(Some(&self.audio_store));
        self.add_column(
            &self.audio_treeview,
            "Stream id",
            ALIGN_LEFT,
            STREAM_ID_DISPLAY_COL,
            Some(200),
        );
        self.add_column(&self.audio_treeview, "Lang.", ALIGN_CENTER, LANGUAGE_COL, None);
        self.add_column(&self.audio_treeview, "Codec", ALIGN_LEFT, CODEC_COL, None);
        self.add_column(&self.audio_treeview, "Rate", ALIGN_RIGHT, AUDIO_RATE_COL, None);
        self.add_column(&self.audio_treeview, "Channels", ALIGN_RIGHT, AUDIO_CHANNELS_COL, None);
        self.add_column(&self.audio_treeview, "Comment", ALIGN_LEFT, COMMENT_COL, None);

        self.text_treeview.set_model(Some(&self.text_store));
        self.add_column(
            &self.text_treeview,
            "Stream id",
            ALIGN_LEFT,
            STREAM_ID_DISPLAY_COL,
            Some(200),
        );
        self.add_column(&self.text_treeview, "Lang.", ALIGN_CENTER, LANGUAGE_COL, None);
        self.add_column(&self.text_treeview, "Codec", ALIGN_LEFT, CODEC_COL, None);
        self.add_column(&self.text_treeview, "Format", ALIGN_LEFT, TEXT_FORMAT_COL, None);
        self.add_column(&self.text_treeview, "Comment", ALIGN_LEFT, COMMENT_COL, None);
    }

    fn add_column(
        &self,
        treeview: &gtk::TreeView,
        title: &str,
        alignment: f32,
        col_id: u32,
        width: Option<i32>,
    ) {
        let col = gtk::TreeViewColumn::new();
        col.set_title(title);

        let renderer = gtk::CellRendererText::new();
        renderer.set_alignment(alignment, 0.5f32);
        col.pack_start(&renderer, true);
        col.add_attribute(&renderer, "text", col_id as i32);

        if let Some(width) = width {
            renderer.set_fixed_size(width, -1);
        }

        treeview.append_column(&col);
    }
}
