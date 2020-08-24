use glib::{clone, GString};
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use crate::spawn;

use super::{
    streams_controller::{EXPORT_FLAG_COL, STREAM_ID_COL},
    MainController, StreamsController, UIDispatcher, UIEventSender, UIFocusContext,
};

macro_rules! on_stream_selected(
    ($main_ctrl_rc:expr, $store:ident, $selected:ident) => (
        {
            let main_ctrl_rc_cb = Rc::clone(&$main_ctrl_rc);
            move |treeview| {
                if let (Some(cursor_path), _) = treeview.get_cursor() {
                    let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
                    let streams_ctrl = &mut main_ctrl.streams_ctrl;

                    if let Some(iter) = streams_ctrl.$store.get_iter(&cursor_path) {
                        let stream = streams_ctrl.stream_at(&streams_ctrl.$store, &iter);
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
                            let streams = streams_ctrl.selected_streams();

                            // Asynchronoulsy notify the main controller
                            let main_ctrl_rc = Rc::clone(&main_ctrl_rc_cb);
                            spawn!(async move {
                                main_ctrl_rc.borrow_mut().select_streams(&streams);
                            });
                        }
                    }
                }
            }
        }
    );
);

macro_rules! register_on_export_toggled(
    ($streams_ctrl:expr, $main_ctrl_rc:expr, $treeview:ident, $store:ident, $streams_getter:ident) => {
        if let Some(col) = $streams_ctrl
            .$treeview
            .get_column(EXPORT_FLAG_COL as i32)
        {
            let mut renderers = col.get_cells();
            debug_assert!(renderers.len() == 1);
            let main_ctrl_rc = $main_ctrl_rc;
            renderers
                .pop()
                .unwrap()
                .downcast::<gtk::CellRendererToggle>()
                .expect("Unexpected `CellRenderer` type for `export` column")
                .connect_toggled(clone!(@strong main_ctrl_rc => move |_, tree_path| {
                    let mut main_ctrl = main_ctrl_rc.borrow_mut();
                    let store = main_ctrl.streams_ctrl.$store.clone();

                    store.get_iter(&tree_path).map(|iter| {
                        let stream_id = store
                            .get_value(&iter, STREAM_ID_COL as i32)
                            .get::<GString>()
                            .unwrap()
                            .unwrap();
                        let value = !store
                            .get_value(&iter, EXPORT_FLAG_COL as i32)
                            .get_some::<bool>()
                            .unwrap();
                        store.set_value(&iter, EXPORT_FLAG_COL, &glib::Value::from(&value));

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
                }));
        }
    };
);

pub struct StreamsDispatcher;
impl UIDispatcher for StreamsDispatcher {
    type Controller = StreamsController;

    fn setup(
        streams_ctrl: &mut StreamsController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        _app: &gtk::Application,
        ui_event: &UIEventSender,
    ) {
        // Video stream selection
        streams_ctrl
            .video_treeview
            .connect_cursor_changed(on_stream_selected!(
                main_ctrl_rc,
                video_store,
                video_selected
            ));

        // Audio stream selection
        streams_ctrl
            .audio_treeview
            .connect_cursor_changed(on_stream_selected!(
                main_ctrl_rc,
                audio_store,
                audio_selected
            ));

        // Text stream selection
        streams_ctrl
            .text_treeview
            .connect_cursor_changed(on_stream_selected!(main_ctrl_rc, text_store, text_selected));

        // Video stream export toggled
        register_on_export_toggled!(
            streams_ctrl,
            main_ctrl_rc,
            video_treeview,
            video_store,
            video
        );

        // Audio stream export toggled
        register_on_export_toggled!(
            streams_ctrl,
            main_ctrl_rc,
            audio_treeview,
            audio_store,
            audio
        );

        // Text stream export toggled
        register_on_export_toggled!(streams_ctrl, main_ctrl_rc, text_treeview, text_store, text);

        let ui_event = ui_event.clone();
        streams_ctrl.page.connect_map(move |_| {
            ui_event.switch_to(UIFocusContext::StreamsPage);
        });
    }
}
