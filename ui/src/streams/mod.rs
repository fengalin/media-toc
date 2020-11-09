mod controller;
pub use self::controller::{ClickedStatus, Controller};

mod dispatcher;
pub use self::dispatcher::Dispatcher;

use crate::UIEventChannel;

#[derive(Debug)]
pub enum Event {
    StreamClicked(gst::StreamType),
    ExportToggled(gst::StreamType, gtk::TreePath),
}

fn stream_clicked(type_: gst::StreamType) {
    UIEventChannel::send(Event::StreamClicked(type_));
}

fn export_toggled(type_: gst::StreamType, tree_path: gtk::TreePath) {
    UIEventChannel::send(Event::ExportToggled(type_, tree_path));
}
