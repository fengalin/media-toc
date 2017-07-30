extern crate gtk;

use gtk::WidgetExt;

use ::media::Context;

pub struct MediaController {
    container: gtk::Widget,
    pub drawingarea: gtk::DrawingArea,
}

impl MediaController {
    pub fn new(container: gtk::Widget, drawingarea: gtk::DrawingArea) -> MediaController {
        MediaController{
            container: container,
            drawingarea: drawingarea,
        }
    }

    pub fn show(&self) {
        self.container.show();
    }

    pub fn hide(&self) {
        self.container.hide();
    }
}
