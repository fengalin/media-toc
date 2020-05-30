use gtk::prelude::*;

use metadata::MediaInfo;

use super::{PlaybackPipeline, UIController};

pub struct PerspectiveController {
    pub(super) menu_btn: gtk::MenuButton,
    pub(super) popover: gtk::PopoverMenu,
    pub(super) stack: gtk::Stack,
    split_btn: gtk::Button,
}

impl PerspectiveController {
    pub fn new(builder: &gtk::Builder) -> Self {
        let mut ctrl = PerspectiveController {
            menu_btn: builder.get_object("perspective-menu-btn").unwrap(),
            popover: builder.get_object("perspective-popovermenu").unwrap(),
            stack: builder.get_object("perspective-stack").unwrap(),
            split_btn: builder.get_object("perspective-split-btn").unwrap(),
        };

        ctrl.cleanup();

        ctrl
    }
}

impl UIController for PerspectiveController {
    fn new_media(&mut self, _pipeline: &PlaybackPipeline) {
        self.menu_btn.set_sensitive(true);
    }

    fn cleanup(&mut self) {
        self.menu_btn.set_sensitive(false);
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.split_btn
            .set_sensitive(info.streams.is_audio_selected());
    }
}
