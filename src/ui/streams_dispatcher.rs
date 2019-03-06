use glib::GString;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use super::{
    streams_controller::{EXPORT_FLAG_COL, STREAM_ID_COL},
    MainController, UIDispatcher,
};

macro_rules! on_stream_selected(
    ($main_ctrl_rc:expr, $store:ident, $tree_path:expr, $selected:ident) => {
        let mut main_ctrl = $main_ctrl_rc.borrow_mut();
        let streams_ctrl = &mut main_ctrl.streams_ctrl;

        if let Some(iter) = streams_ctrl.$store.get_iter($tree_path) {
            let stream = streams_ctrl.get_stream_at(&streams_ctrl.$store, &iter);
            let stream_to_select = match streams_ctrl.$selected {
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
                streams_ctrl.$selected = Some(new_stream);
                let streams = streams_ctrl.get_selected_streams();

                // Asynchronoulsy notify the main controller
                let main_ctrl_weak = Rc::downgrade(&$main_ctrl_rc);
                gtk::idle_add(move || {
                    let main_ctrl_rc = main_ctrl_weak.upgrade().unwrap();
                    main_ctrl_rc.borrow_mut().select_streams(&streams);
                    glib::Continue(false)
                });
            }
        }
    };
);

macro_rules! on_export_toggled(
    ($main_ctrl_rc:expr, $store:ident, $tree_path:expr, $streams_getter:ident) => {
        let mut main_ctrl = $main_ctrl_rc.borrow_mut();
        let store = main_ctrl.streams_ctrl.$store.clone();

        store.get_iter(&$tree_path).map(|iter| {
            let stream_id = store
                .get_value(&iter, STREAM_ID_COL as i32)
                .get::<GString>()
                .unwrap();
            let value = !store
                .get_value(&iter, EXPORT_FLAG_COL as i32)
                .get::<bool>()
                .unwrap();
            store.set_value(&iter, EXPORT_FLAG_COL, &gtk::Value::from(&value));

            if let Some(pipeline) = main_ctrl.pipeline.as_mut() {
                pipeline
                    .info
                    .write()
                    .unwrap()
                    .streams
                    .$streams_getter(stream_id)
                    .as_mut()
                    .unwrap()
                    .must_export = value;
            }
        });
    };
);

pub struct StreamsDispatcher;
impl UIDispatcher for StreamsDispatcher {
    fn setup(_gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>) {
        let main_ctrl = main_ctrl_rc.borrow();
        let streams_ctrl = &main_ctrl.streams_ctrl;

        // Video stream selection
        let main_ctrl_clone = Rc::clone(main_ctrl_rc);
        streams_ctrl
            .video_treeview
            .connect_row_activated(move |_, tree_path, _| {
                on_stream_selected!(main_ctrl_clone, video_store, tree_path, video_selected);
            });

        // Audio stream selection
        let main_ctrl_clone = Rc::clone(main_ctrl_rc);
        streams_ctrl
            .audio_treeview
            .connect_row_activated(move |_, tree_path, _| {
                on_stream_selected!(main_ctrl_clone, audio_store, tree_path, audio_selected);
            });

        // Text stream selection
        let main_ctrl_clone = Rc::clone(main_ctrl_rc);
        streams_ctrl
            .text_treeview
            .connect_row_activated(move |_, tree_path, _| {
                on_stream_selected!(main_ctrl_clone, text_store, tree_path, text_selected);
            });

        // Video stream export toggled
        let main_ctrl_clone = Rc::clone(main_ctrl_rc);
        if let Some(col) = streams_ctrl
            .video_treeview
            .get_column(EXPORT_FLAG_COL as i32)
        {
            let mut renderers = col.get_cells();
            debug_assert!(renderers.len() == 1);
            renderers
                .pop()
                .unwrap()
                .downcast::<gtk::CellRendererToggle>()
                .expect("Unexpected `CellRenderer` type for `export` column")
                .connect_toggled(move |_, tree_path| {
                    on_export_toggled!(main_ctrl_clone, video_store, tree_path, get_video_mut);
                });
        }

        // Audio stream export toggled
        let main_ctrl_clone = Rc::clone(main_ctrl_rc);
        if let Some(col) = streams_ctrl
            .audio_treeview
            .get_column(EXPORT_FLAG_COL as i32)
        {
            let mut renderers = col.get_cells();
            debug_assert!(renderers.len() == 1);
            renderers
                .pop()
                .unwrap()
                .downcast::<gtk::CellRendererToggle>()
                .expect("Unexpected `CellRenderer` type for `export` column")
                .connect_toggled(move |_, tree_path| {
                    on_export_toggled!(main_ctrl_clone, audio_store, tree_path, get_audio_mut);
                });
        }

        // Text stream export toggled
        let main_ctrl_clone = Rc::clone(main_ctrl_rc);
        if let Some(col) = streams_ctrl
            .text_treeview
            .get_column(EXPORT_FLAG_COL as i32)
        {
            let mut renderers = col.get_cells();
            debug_assert!(renderers.len() == 1);
            renderers
                .pop()
                .unwrap()
                .downcast::<gtk::CellRendererToggle>()
                .expect("Unexpected `CellRenderer` type for `export` column")
                .connect_toggled(move |_, tree_path| {
                    on_export_toggled!(main_ctrl_clone, text_store, tree_path, get_text_mut);
                });
        }
    }
}
