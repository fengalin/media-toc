use glib::clone;
use gtk::prelude::*;

use super::{StreamsController, UIDispatcher, UIEventSender, UIFocusContext};

pub struct StreamsDispatcher;
impl UIDispatcher for StreamsDispatcher {
    type Controller = StreamsController;

    fn setup(
        streams_ctrl: &mut StreamsController,
        _app: &gtk::Application,
        ui_event: &UIEventSender,
    ) {
        streams_ctrl.video.treeview.connect_cursor_changed(
            clone!(@strong ui_event => move |_| ui_event.stream_clicked(gst::StreamType::VIDEO)),
        );
        streams_ctrl.video.export_renderer().connect_toggled(
            clone!(@strong ui_event => move |_, tree_path| {
                ui_event.stream_export_toggled(gst::StreamType::VIDEO, tree_path)
            }),
        );

        streams_ctrl.audio.treeview.connect_cursor_changed(
            clone!(@strong ui_event => move |_| ui_event.stream_clicked(gst::StreamType::AUDIO)),
        );
        streams_ctrl.audio.export_renderer().connect_toggled(
            clone!(@strong ui_event => move |_, tree_path| {
                ui_event.stream_export_toggled(gst::StreamType::AUDIO, tree_path)
            }),
        );

        streams_ctrl.text.treeview.connect_cursor_changed(
            clone!(@strong ui_event => move |_| ui_event.stream_clicked(gst::StreamType::TEXT)),
        );
        streams_ctrl.text.export_renderer().connect_toggled(
            clone!(@strong ui_event => move |_, tree_path| {
                ui_event.stream_export_toggled(gst::StreamType::TEXT, tree_path)
            }),
        );

        streams_ctrl
            .page
            .connect_map(clone!(@strong ui_event => move |_| {
                ui_event.switch_to(UIFocusContext::StreamsPage);
            }));
    }
}
