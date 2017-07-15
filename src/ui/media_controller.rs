extern crate gtk;
extern crate ffmpeg;

use gtk::WidgetExt;

use ::media::Context;

pub struct MediaController {
    container: gtk::Grid,
}

impl MediaController {
    pub fn new(container: gtk::Grid) -> MediaController {
        MediaController{ container: container }
    }

    pub fn show(&self) {
        self.container.show();
    }

    pub fn hide(&self) {
        self.container.hide();
    }
}

pub trait MediaNotifiable {
    fn new_media(&mut self, &Context);
}
