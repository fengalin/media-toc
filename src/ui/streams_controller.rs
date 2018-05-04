use std::rc::{Rc, Weak};
use std::cell::RefCell;

use gettextrs::gettext;
use glib;
use gstreamer as gst;

use gtk;
use gtk::prelude::*;

use media::PlaybackContext;

use metadata::Stream;

use super::MainController;

const ALIGN_LEFT: f32 = 0f32;
const ALIGN_CENTER: f32 = 0.5f32;
const ALIGN_RIGHT: f32 = 1f32;

const EXPORT_FLAG_COL: u32 = 0;
const STREAM_ID_COL: u32 = 1;
const STREAM_ID_DISPLAY_COL: u32 = 2;
const LANGUAGE_COL: u32 = 3;
const CODEC_COL: u32 = 4;
const COMMENT_COL: u32 = 5;

const VIDEO_WIDTH_COL: u32 = 6;
const VIDEO_HEIGHT_COL: u32 = 7;

const AUDIO_RATE_COL: u32 = 6;
const AUDIO_CHANNELS_COL: u32 = 7;

const TEXT_FORMAT_COL: u32 = 6;

macro_rules! on_stream_selected(
    ($this:expr, $store:expr, $tree_path:expr, $selected:expr) => {
        if let Some(iter) = $store.get_iter($tree_path) {
            let stream = $this.get_stream_at(&$store, &iter);
            let stream_to_select = match $selected {
                Some(ref stream_id) => {
                    if stream_id != &stream {
                        // Stream has changed
                        Some(stream)
                    } else {
                        None
                    }
                }
                None => Some(stream),
            };
            if let Some(new_stream) = stream_to_select {
                $selected = Some(new_stream);
                $this.trigger_stream_selection();
            }
        }
    };
);

pub struct StreamsController {
    video_treeview: gtk::TreeView,
    video_store: gtk::ListStore,
    video_selected: Option<String>,

    audio_treeview: gtk::TreeView,
    audio_store: gtk::ListStore,
    audio_selected: Option<String>,

    text_treeview: gtk::TreeView,
    text_store: gtk::ListStore,
    text_selected: Option<String>,

    main_ctrl: Option<Weak<RefCell<MainController>>>,
}

impl StreamsController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this_rc = Rc::new(RefCell::new(StreamsController {
            video_treeview: builder.get_object("video_streams-treeview").unwrap(),
            video_store: builder.get_object("video_streams-liststore").unwrap(),
            video_selected: None,

            audio_treeview: builder.get_object("audio_streams-treeview").unwrap(),
            audio_store: builder.get_object("audio_streams-liststore").unwrap(),
            audio_selected: None,

            text_treeview: builder.get_object("text_streams-treeview").unwrap(),
            text_store: builder.get_object("text_streams-liststore").unwrap(),
            text_selected: None,

            main_ctrl: None,
        }));

        {
            let this_clone = Rc::clone(&this_rc);
            let mut this = this_rc.borrow_mut();
            this.cleanup();
            this.init_treeviews(this_clone);
        }

        this_rc
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();

        this.main_ctrl = Some(Rc::downgrade(main_ctrl));

        // Video stream selection
        let this_clone = Rc::clone(this_rc);
        this.video_treeview
            .connect_row_activated(move |_, tree_path, _| {
                let mut this = this_clone.borrow_mut();
                on_stream_selected!(this, this.video_store, tree_path, this.video_selected);
            });

        // Audio stream selection
        let this_clone = Rc::clone(this_rc);
        this.audio_treeview
            .connect_row_activated(move |_, tree_path, _| {
                let mut this = this_clone.borrow_mut();
                on_stream_selected!(this, this.audio_store, tree_path, this.audio_selected);
            });

        // Text stream selection
        let this_clone = Rc::clone(this_rc);
        this.text_treeview
            .connect_row_activated(move |_, tree_path, _| {
                let mut this = this_clone.borrow_mut();
                on_stream_selected!(this, this.text_store, tree_path, this.text_selected);
            });
    }

    fn toggle_export(store: &gtk::ListStore, tree_path: gtk::TreePath) -> Option<bool> {
        store.get_iter(&tree_path).map(|iter| {
            let value = !store.get_value(&iter, EXPORT_FLAG_COL as i32).get::<bool>().unwrap();
            store.set_value(&iter, EXPORT_FLAG_COL, &gtk::Value::from(&value));
            value
        })
    }

    fn video_export_toggled(&self, tree_path: gtk::TreePath) {
        if let Some(value) = Self::toggle_export(&self.video_store, tree_path) {
            // TODO: update MediaInfo
            println!("video export: {}", value);
        }
    }

    fn audio_export_toggled(&self, tree_path: gtk::TreePath) {
        if let Some(value) = Self::toggle_export(&self.audio_store, tree_path) {
            // TODO: update MediaInfo
            println!("audio export: {}", value);
        }
    }

    fn text_export_toggled(&self, tree_path: gtk::TreePath) {
        if let Some(value) = Self::toggle_export(&self.text_store, tree_path) {
            // TODO: update MediaInfo
            println!("text export: {}", value);
        }
    }

    pub fn cleanup(&mut self) {
        self.video_store.clear();
        self.video_selected = None;
        self.audio_store.clear();
        self.audio_selected = None;
        self.text_store.clear();
        self.text_selected = None;
    }

    pub fn new_media(&mut self, context: &PlaybackContext) {
        let info = context.info.lock().unwrap();

        // Video streams
        for stream in &info.streams.video {
            let iter = self.add_stream(&self.video_store, stream);
            let caps_structure = stream.caps.get_structure(0).unwrap();
            if let Some(width) = caps_structure.get::<i32>("width") {
                self.video_store
                    .set_value(&iter, VIDEO_WIDTH_COL, &gtk::Value::from(&width));
            }
            if let Some(height) = caps_structure.get::<i32>("height") {
                self.video_store
                    .set_value(&iter, VIDEO_HEIGHT_COL, &gtk::Value::from(&height));
            }
        }

        self.video_selected = match self.video_store.get_iter_first() {
            Some(ref iter) => {
                self.video_treeview.get_selection().select_iter(iter);
                Some(self.get_stream_at(&self.video_store, iter))
            }
            None => None,
        };

        // Audio streams
        for stream in &info.streams.audio {
            let iter = self.add_stream(&self.audio_store, stream);
            let caps_structure = stream.caps.get_structure(0).unwrap();
            if let Some(rate) = caps_structure.get::<i32>("rate") {
                self.audio_store
                    .set_value(&iter, AUDIO_RATE_COL, &gtk::Value::from(&rate));
            }
            if let Some(channels) = caps_structure.get::<i32>("channels") {
                self.audio_store
                    .set_value(&iter, AUDIO_CHANNELS_COL, &gtk::Value::from(&channels));
            }
        }

        self.audio_selected = match self.audio_store.get_iter_first() {
            Some(ref iter) => {
                self.audio_treeview.get_selection().select_iter(iter);
                Some(self.get_stream_at(&self.audio_store, iter))
            }
            None => None,
        };

        // Text streams
        for stream in &info.streams.text {
            let iter = self.add_stream(&self.text_store, stream);
            let caps_structure = stream.caps.get_structure(0).unwrap();
            if let Some(format) = caps_structure.get::<&str>("format") {
                self.text_store
                    .set_value(&iter, TEXT_FORMAT_COL, &gtk::Value::from(&format));
            }
        }

        self.text_selected = match self.text_store.get_iter_first() {
            Some(ref iter) => {
                self.text_treeview.get_selection().select_iter(iter);
                Some(self.get_stream_at(&self.text_store, iter))
            }
            None => None,
        };
    }

    pub fn trigger_stream_selection(&self) {
        // Asynchronoulsy notify the main controller
        let main_ctrl_weak = Weak::clone(self.main_ctrl.as_ref().unwrap());
        let mut streams: Vec<String> = Vec::new();
        if let Some(stream) = self.video_selected.as_ref() {
            streams.push(stream.clone());
        }
        if let Some(stream) = self.audio_selected.as_ref() {
            streams.push(stream.clone());
        }
        if let Some(stream) = self.text_selected.as_ref() {
            streams.push(stream.clone());
        }
        gtk::idle_add(move || {
            let main_ctrl_rc = main_ctrl_weak.upgrade().unwrap();
            main_ctrl_rc.borrow_mut().select_streams(&streams);
            glib::Continue(false)
        });
    }

    fn add_stream(&self, store: &gtk::ListStore, stream: &Stream) -> gtk::TreeIter {
        let id_parts: Vec<&str> = stream.id.split('/').collect();
        let stream_id_display = if id_parts.len() == 2 {
            id_parts[1].to_owned()
        } else {
            gettext("unknown")
        };

        let iter = store.insert_with_values(
            None,
            &[EXPORT_FLAG_COL, STREAM_ID_COL, STREAM_ID_DISPLAY_COL],
            &[&true, &stream.id, &stream_id_display],
        );

        if let Some(ref tags) = stream.tags {
            let language = match tags.get_index::<gst::tags::LanguageName>(0) {
                Some(ref language) => language.get().unwrap(),
                None => match tags.get_index::<gst::tags::LanguageCode>(0) {
                    Some(ref language) => language.get().unwrap(),
                    None => "-",
                },
            };
            store.set_value(&iter, LANGUAGE_COL, &gtk::Value::from(language));

            if let Some(ref comment) = tags.get_index::<gst::tags::Comment>(0) {
                store.set_value(
                    &iter,
                    COMMENT_COL,
                    &gtk::Value::from(comment.get().unwrap()),
                );
            }
        }

        store.set_value(&iter, CODEC_COL, &gtk::Value::from(&stream.codec_printable));

        iter
    }

    fn get_stream_at(&self, store: &gtk::ListStore, iter: &gtk::TreeIter) -> String {
        store
            .get_value(iter, STREAM_ID_COL as i32)
            .get::<String>()
            .unwrap()
    }

    fn init_treeviews(&self, this_rc: Rc<RefCell<Self>>) {
        self.video_treeview.set_model(Some(&self.video_store));

        let export_flag_lbl = gettext("Export?");
        let stream_id_lbl = gettext("Stream id");
        let language_lbl = gettext("Language");
        let codec_lbl = gettext("Codec");
        let comment_lbl = gettext("Comment");

        // Video
        let renderer = self.add_check_column(
            &self.video_treeview,
            &export_flag_lbl,
            EXPORT_FLAG_COL,
        );
        let this_clone = Rc::clone(&this_rc);
        renderer.connect_toggled(move |_, tree_path| {
            this_clone.borrow().video_export_toggled(tree_path);
        });
        self.add_text_column(
            &self.video_treeview,
            &stream_id_lbl,
            ALIGN_LEFT,
            STREAM_ID_DISPLAY_COL,
            Some(200),
        );
        self.add_text_column(
            &self.video_treeview,
            &language_lbl,
            ALIGN_CENTER,
            LANGUAGE_COL,
            None,
        );
        self.add_text_column(
            &self.video_treeview,
            &codec_lbl,
            ALIGN_LEFT,
            CODEC_COL,
            None,
        );
        self.add_text_column(
            &self.video_treeview,
            &gettext("Width"),
            ALIGN_RIGHT,
            VIDEO_WIDTH_COL,
            None,
        );
        self.add_text_column(
            &self.video_treeview,
            &gettext("Height"),
            ALIGN_RIGHT,
            VIDEO_HEIGHT_COL,
            None,
        );
        self.add_text_column(
            &self.video_treeview,
            &comment_lbl,
            ALIGN_LEFT,
            COMMENT_COL,
            None,
        );

        // Audio
        self.audio_treeview.set_model(Some(&self.audio_store));
        let renderer = self.add_check_column(
            &self.audio_treeview,
            &export_flag_lbl,
            EXPORT_FLAG_COL,
        );
        let this_clone = Rc::clone(&this_rc);
        renderer.connect_toggled(move |_, tree_path| {
            this_clone.borrow().audio_export_toggled(tree_path);
        });
        self.add_text_column(
            &self.audio_treeview,
            &stream_id_lbl,
            ALIGN_LEFT,
            STREAM_ID_DISPLAY_COL,
            Some(200),
        );
        self.add_text_column(
            &self.audio_treeview,
            &language_lbl,
            ALIGN_CENTER,
            LANGUAGE_COL,
            None,
        );
        self.add_text_column(
            &self.audio_treeview,
            &codec_lbl,
            ALIGN_LEFT,
            CODEC_COL,
            None,
        );
        self.add_text_column(
            &self.audio_treeview,
            &gettext("Rate"),
            ALIGN_RIGHT,
            AUDIO_RATE_COL,
            None,
        );
        self.add_text_column(
            &self.audio_treeview,
            &gettext("Channels"),
            ALIGN_RIGHT,
            AUDIO_CHANNELS_COL,
            None,
        );
        self.add_text_column(
            &self.audio_treeview,
            &comment_lbl,
            ALIGN_LEFT,
            COMMENT_COL,
            None,
        );

        // Text
        self.text_treeview.set_model(Some(&self.text_store));
        let renderer = self.add_check_column(
            &self.text_treeview,
            &export_flag_lbl,
            EXPORT_FLAG_COL,
        );
        let this_clone = Rc::clone(&this_rc);
        renderer.connect_toggled(move |_, tree_path| {
            this_clone.borrow().text_export_toggled(tree_path);
        });
        self.add_text_column(
            &self.text_treeview,
            &stream_id_lbl,
            ALIGN_LEFT,
            STREAM_ID_DISPLAY_COL,
            Some(200),
        );
        self.add_text_column(
            &self.text_treeview,
            &language_lbl,
            ALIGN_CENTER,
            LANGUAGE_COL,
            None,
        );
        self.add_text_column(&self.text_treeview, &codec_lbl, ALIGN_LEFT, CODEC_COL, None);
        self.add_text_column(
            &self.text_treeview,
            &gettext("Format"),
            ALIGN_LEFT,
            TEXT_FORMAT_COL,
            None,
        );
        self.add_text_column(
            &self.text_treeview,
            &comment_lbl,
            ALIGN_LEFT,
            COMMENT_COL,
            None,
        );
    }

    fn add_text_column(
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

    fn add_check_column(
        &self,
        treeview: &gtk::TreeView,
        title: &str,
        col_id: u32,
    ) -> gtk::CellRendererToggle
    {
        let col = gtk::TreeViewColumn::new();
        col.set_title(title);

        let renderer = gtk::CellRendererToggle::new();
        renderer.set_radio(false);
        renderer.set_activatable(true);
        col.pack_start(&renderer, true);
        col.add_attribute(&renderer, "active", col_id as i32);

        treeview.append_column(&col);
        renderer
    }
}
