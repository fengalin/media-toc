extern crate gtk;

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

pub trait NotifiableMedia {
    fn notify_new_media(&mut self, &mut Context);
}
