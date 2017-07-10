extern crate gtk;
extern crate ffmpeg;

use gtk::WidgetExt;

use ::media::Context;

pub struct MediaController {
    container: gtk::Grid,
    stream_index: Option<usize>,
}

impl MediaController {
    pub fn new(container: gtk::Grid) -> MediaController {
        MediaController{ container: container, stream_index: None }
    }

    // FIXME: are there any annotations for setters/getters?
    pub fn set_index(&mut self, index: usize) {
        self.stream_index = Some(index);
    }

    pub fn stream_index(&self) -> Option<usize> {
        self.stream_index
    }

    pub fn show(&self) {
        self.container.show();
    }

    pub fn hide(&self) {
        self.container.hide();
    }
}

pub trait MediaNotifiable {
    fn new_media(&mut self, &mut Context);
}
