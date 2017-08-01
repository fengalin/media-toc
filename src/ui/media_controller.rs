extern crate gtk;

use gtk::WidgetExt;

use ::media::Context;

pub trait MediaHandler {
    fn new_media(&mut self, context: &Context);
}

pub struct MediaController {
    container: gtk::Widget,
    pub drawingarea: gtk::DrawingArea,
    pub draw_handler: u64,
}

impl MediaController {
    pub fn new(container: gtk::Widget, drawingarea: gtk::DrawingArea) -> Self {
        MediaController{
            container: container,
            drawingarea: drawingarea,
            draw_handler: 0,
        }
    }

    pub fn show(&self) {
        self.container.show();
    }

    pub fn hide(&self) {
        self.container.hide();
    }
}
