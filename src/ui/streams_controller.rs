extern crate gstreamer as gst;

extern crate gtk;
use gtk::prelude::*;

use std::collections::HashMap;

use std::rc::{Rc, Weak};
use std::cell::RefCell;

use media::PlaybackContext;

use super::MainController;

const STREAM_ID_COL: i32 = 0;
const CODEC_COL: i32 = 1;

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

        self.add_streams(&self.video_treeview, &self.video_store, &info.video_streams);
        self.add_streams(&self.audio_treeview, &self.audio_store, &info.audio_streams);
        self.add_streams(&self.text_treeview, &self.text_store, &info.text_streams);
    }

    fn add_streams(
        &self,
        treeview: &gtk::TreeView,
        store: &gtk::ListStore,
        streams: &HashMap<String, gst::Caps>,
    ) {
        for (stream_id, caps) in streams {
            let caps_structure = caps.get_structure(0).unwrap();
            store.insert_with_values(
                None,
                &[STREAM_ID_COL as u32, CODEC_COL as u32],
                &[&stream_id, &(caps_structure.get_name())],
            );
        }

        match store.get_iter_first() {
            Some(ref iter) => treeview.get_selection().select_iter(iter),
            None => (),
        }
    }

    pub fn cleanup(&mut self) {
        self.streams_button.set_sensitive(false);
        self.video_store.clear();
    }

    fn init_treeviews(&self) {
        self.video_treeview.set_model(Some(&self.video_store));
        self.add_column(&self.video_treeview, "Stream id", STREAM_ID_COL);
        self.add_column(&self.video_treeview, "Codec", CODEC_COL);

        self.audio_treeview.set_model(Some(&self.audio_store));
        self.add_column(&self.audio_treeview, "Stream id", STREAM_ID_COL);
        self.add_column(&self.audio_treeview, "Codec", CODEC_COL);

        self.text_treeview.set_model(Some(&self.text_store));
        self.add_column(&self.text_treeview, "Stream id", STREAM_ID_COL);
        self.add_column(&self.text_treeview, "Codec", CODEC_COL);
    }

    fn add_column(
        &self,
        treeview: &gtk::TreeView,
        title: &str,
        col_id: i32,
    ) {
        let col = gtk::TreeViewColumn::new();
        col.set_title(title);

        let renderer = gtk::CellRendererText::new();
        col.pack_start(&renderer, true);
        col.add_attribute(&renderer, "text", col_id);
        if col_id == STREAM_ID_COL {
            col.set_max_width(550);
        }

        treeview.append_column(&col);
    }
}
