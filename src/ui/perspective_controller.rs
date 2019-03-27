use gtk;
use gtk::prelude::*;

use crate::{application::CommandLineArguments, media::PlaybackPipeline, metadata::MediaInfo};

use super::UIController;

pub struct PerspectiveController {
    pub(super) menu_btn: gtk::MenuButton,
    pub(super) popover: gtk::PopoverMenu,
    pub(super) stack: gtk::Stack,
    split_btn: gtk::Button,
}

impl PerspectiveController {
    pub fn new(builder: &gtk::Builder) -> Self {
        PerspectiveController {
            menu_btn: builder.get_object("perspective-menu-btn").unwrap(),
            popover: builder.get_object("perspective-popovermenu").unwrap(),
            stack: builder.get_object("perspective-stack").unwrap(),
            split_btn: builder.get_object("perspective-split-btn").unwrap(),
        }
    }
}

impl UIController for PerspectiveController {
    fn setup(&mut self, _args: &CommandLineArguments) {
        self.cleanup();
    }

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
