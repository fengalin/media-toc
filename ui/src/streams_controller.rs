use gettextrs::gettext;
use gstreamer as gst;

use gtk::prelude::*;

use std::sync::Arc;

use metadata::Stream;

use crate::spawn;

use super::{PlaybackPipeline, UIController};

const ALIGN_LEFT: f32 = 0f32;
const ALIGN_CENTER: f32 = 0.5f32;
const ALIGN_RIGHT: f32 = 1f32;

pub(super) const EXPORT_FLAG_COL: u32 = 0;
pub(super) const STREAM_ID_COL: u32 = 1;
const STREAM_ID_DISPLAY_COL: u32 = 2;
const LANGUAGE_COL: u32 = 3;
const CODEC_COL: u32 = 4;
const COMMENT_COL: u32 = 5;

const VIDEO_WIDTH_COL: u32 = 6;
const VIDEO_HEIGHT_COL: u32 = 7;

const AUDIO_RATE_COL: u32 = 6;
const AUDIO_CHANNELS_COL: u32 = 7;

const TEXT_FORMAT_COL: u32 = 6;

pub struct StreamsController {
    pub(super) page: gtk::Grid,

    pub(super) video_treeview: gtk::TreeView,
    pub(super) video_store: gtk::ListStore,
    pub(super) video_selected: Option<Arc<str>>,

    pub(super) audio_treeview: gtk::TreeView,
    pub(super) audio_store: gtk::ListStore,
    pub(super) audio_selected: Option<Arc<str>>,

    pub(super) text_treeview: gtk::TreeView,
    pub(super) text_store: gtk::ListStore,
    pub(super) text_selected: Option<Arc<str>>,
}

impl UIController for StreamsController {
    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        {
            let mut info = pipeline.info.write().unwrap();

            // Video streams
            let mut sorted_ids = info
                .streams
                .video
                .keys()
                .map(|key| Arc::clone(key))
                .collect::<Vec<Arc<str>>>();
            sorted_ids.sort();
            for stream_id in sorted_ids {
                let stream = info.streams.get_video_mut(stream_id).unwrap();
                stream.must_export = true;
                let iter = self.add_stream(&self.video_store, stream);
                let caps_structure = stream.caps.get_structure(0).unwrap();
                if let Ok(Some(width)) = caps_structure.get::<i32>("width") {
                    self.video_store
                        .set_value(&iter, VIDEO_WIDTH_COL, &glib::Value::from(&width));
                }
                if let Ok(Some(height)) = caps_structure.get::<i32>("height") {
                    self.video_store.set_value(
                        &iter,
                        VIDEO_HEIGHT_COL,
                        &glib::Value::from(&height),
                    );
                }
            }

            // Audio streams
            let mut sorted_ids = info
                .streams
                .audio
                .keys()
                .map(|key| Arc::clone(key))
                .collect::<Vec<Arc<str>>>();
            sorted_ids.sort();
            for stream_id in sorted_ids {
                let stream = info.streams.get_audio_mut(stream_id).unwrap();
                stream.must_export = true;
                let iter = self.add_stream(&self.audio_store, stream);
                let caps_structure = stream.caps.get_structure(0).unwrap();
                if let Ok(Some(rate)) = caps_structure.get::<i32>("rate") {
                    self.audio_store
                        .set_value(&iter, AUDIO_RATE_COL, &glib::Value::from(&rate));
                }
                if let Ok(Some(channels)) = caps_structure.get::<i32>("channels") {
                    self.audio_store.set_value(
                        &iter,
                        AUDIO_CHANNELS_COL,
                        &glib::Value::from(&channels),
                    );
                }
            }

            // Text streams
            let mut sorted_ids = info
                .streams
                .text
                .keys()
                .map(|key| Arc::clone(key))
                .collect::<Vec<Arc<str>>>();
            sorted_ids.sort();
            for stream_id in sorted_ids {
                let stream = info.streams.get_text_mut(stream_id).unwrap();
                let iter = self.add_stream(&self.text_store, stream);
                // FIXME: discard text stream export for now as it hangs the export
                // (see https://github.com/fengalin/media-toc/issues/136)
                stream.must_export = false;
                self.text_store
                    .set_value(&iter, EXPORT_FLAG_COL, &glib::Value::from(&false));
                let caps_structure = stream.caps.get_structure(0).unwrap();
                if let Ok(Some(format)) = caps_structure.get::<&str>("format") {
                    self.text_store
                        .set_value(&iter, TEXT_FORMAT_COL, &glib::Value::from(&format));
                }
            }
        }

        self.video_selected = self.video_store.get_iter_first().map(|ref iter| {
            self.video_treeview.get_selection().select_iter(iter);
            self.get_stream_at(&self.video_store, iter)
        });

        self.audio_selected = self.audio_store.get_iter_first().map(|ref iter| {
            self.audio_treeview.get_selection().select_iter(iter);
            self.get_stream_at(&self.audio_store, iter)
        });

        self.text_selected = self.text_store.get_iter_first().map(|ref iter| {
            self.text_treeview.get_selection().select_iter(iter);
            self.get_stream_at(&self.text_store, iter)
        });
    }

    fn cleanup(&mut self) {
        self.video_store.clear();
        self.video_selected = None;
        self.audio_store.clear();
        self.audio_selected = None;
        self.text_store.clear();
        self.text_selected = None;
    }

    fn grab_focus(&self) {
        // grab focus asynchronoulsy because it triggers the `cursor_changed` signal
        // which needs to check if the stream has changed
        let audio_treeview = self.audio_treeview.clone();
        spawn!(async move {
            audio_treeview.grab_focus();
        });
    }
}

impl StreamsController {
    pub fn new(builder: &gtk::Builder) -> Self {
        let mut ctrl = StreamsController {
            page: builder.get_object("streams-grid").unwrap(),

            video_treeview: builder.get_object("video_streams-treeview").unwrap(),
            video_store: builder.get_object("video_streams-liststore").unwrap(),
            video_selected: None,

            audio_treeview: builder.get_object("audio_streams-treeview").unwrap(),
            audio_store: builder.get_object("audio_streams-liststore").unwrap(),
            audio_selected: None,

            text_treeview: builder.get_object("text_streams-treeview").unwrap(),
            text_store: builder.get_object("text_streams-liststore").unwrap(),
            text_selected: None,
        };

        ctrl.cleanup();
        ctrl.init_treeviews();

        ctrl
    }

    pub fn get_selected_streams(&self) -> Vec<Arc<str>> {
        let mut streams: Vec<Arc<str>> = Vec::new();
        if let Some(stream) = self.video_selected.as_ref() {
            streams.push(Arc::clone(stream));
        }
        if let Some(stream) = self.audio_selected.as_ref() {
            streams.push(Arc::clone(stream));
        }
        if let Some(stream) = self.text_selected.as_ref() {
            streams.push(Arc::clone(stream));
        }

        streams
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
            &[&true, &stream.id.as_ref(), &stream_id_display],
        );

        let lang = stream
            .tags
            .get_index::<gst::tags::LanguageName>(0)
            .or_else(|| stream.tags.get_index::<gst::tags::LanguageCode>(0))
            .and_then(glib::TypedValue::get)
            .unwrap_or("-");
        store.set_value(&iter, LANGUAGE_COL, &glib::Value::from(lang));

        if let Some(comment) = stream
            .tags
            .get_index::<gst::tags::Comment>(0)
            .and_then(glib::TypedValue::get)
        {
            store.set_value(&iter, COMMENT_COL, &glib::Value::from(comment));
        }

        store.set_value(
            &iter,
            CODEC_COL,
            &glib::Value::from(&stream.codec_printable),
        );

        iter
    }

    pub fn get_stream_at(&self, store: &gtk::ListStore, iter: &gtk::TreeIter) -> Arc<str> {
        store
            .get_value(iter, STREAM_ID_COL as i32)
            .get::<String>()
            .unwrap()
            .unwrap()
            .into()
    }

    fn init_treeviews(&self) {
        self.video_treeview.set_model(Some(&self.video_store));

        let export_flag_lbl = gettext("Export?");
        let stream_id_lbl = gettext("Stream id");
        let language_lbl = gettext("Language");
        let codec_lbl = gettext("Codec");
        let comment_lbl = gettext("Comment");

        // Video
        self.add_check_column(&self.video_treeview, &export_flag_lbl, EXPORT_FLAG_COL);
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
        self.add_check_column(&self.audio_treeview, &export_flag_lbl, EXPORT_FLAG_COL);
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
        let renderer =
            self.add_check_column(&self.text_treeview, &export_flag_lbl, EXPORT_FLAG_COL);
        // FIXME: discard text stream export for now as it hangs the export
        // (see https://github.com/fengalin/media-toc/issues/136)
        renderer.set_activatable(false);

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
    ) -> gtk::CellRendererToggle {
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
