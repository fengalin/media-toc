extern crate cairo;

extern crate gtk;
use gtk::prelude::*;

extern crate glib;

use std::rc::{Rc, Weak};
use std::cell::RefCell;

use media::PlaybackContext;

//use metadata;

use super::MainController;

pub struct StreamsController {
    streams_button: gtk::ToggleButton,
    display_streams_stack: gtk::Stack,

    main_ctrl: Option<Weak<RefCell<MainController>>>,
}

impl StreamsController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this_rc = Rc::new(RefCell::new(StreamsController {
            streams_button: builder.get_object("streams-toggle").unwrap(),
            display_streams_stack: builder.get_object("display_streams-stack").unwrap(),

            main_ctrl: None,
        }));

        {
            let mut this = this_rc.borrow_mut();
            this.cleanup();
        }

        this_rc
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();

        this.main_ctrl = Some(Rc::downgrade(main_ctrl));

        // streams button
        let this_clone = Rc::clone(this_rc);
        this.streams_button.connect_clicked(move |button| {
            let page_name = if button.get_active() {
                "streams".into()
            } else {
                "display".into()
            };
            this_clone.borrow_mut().display_streams_stack.set_visible_child_name(page_name);
        });
    }

    pub fn new_media(&mut self, _context: &PlaybackContext) {
        self.streams_button.set_sensitive(true);
    }

    pub fn cleanup(&mut self) {
        println!("cleanup");
        self.streams_button.set_sensitive(false);
    }
}
