use gtk::prelude::*;

use media::PlaybackPipeline;
use metadata::MediaInfo;

use crate::prelude::*;

pub struct Controller {
    pub(super) menu_btn: gtk::MenuButton,
    pub(super) popover: gtk::PopoverMenu,
    pub(super) stack: gtk::Stack,
    split_btn: gtk::Button,
}

impl Controller {
    pub fn new(builder: &gtk::Builder) -> Self {
        let mut ctrl = Controller {
            menu_btn: builder.object("perspective-menu-btn").unwrap(),
            popover: builder.object("perspective-popovermenu").unwrap(),
            stack: builder.object("perspective-stack").unwrap(),
            split_btn: builder.object("perspective-split-btn").unwrap(),
        };

        ctrl.cleanup();

        ctrl
    }
}

impl UIController for Controller {
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
