use glib::Cast;

use gtk;
use gtk::{BinExt, ButtonExt, ContainerExt, ImageExt, StackExt, WidgetExt};

use std::rc::Rc;
use std::cell::RefCell;

use metadata::MediaInfo;

use super::{MainController, PlaybackContext};

macro_rules! gtk_downcast(
    ($source:expr, $target_type:ty, $item_name:expr) => {
        $source.clone()
            .downcast::<$target_type>()
            .expect(&format!(concat!("PerspectiveController ",
                    "unexpected type for perspective item {:?}",
                ),
                $item_name,
            ))
    };
    ($source:expr, $item_index:expr, $target_type:ty, $item_name:expr) => {
        $source.get_children()
            .get($item_index)
            .expect(&format!("PerspectiveController no child at index {} for perspective item {:?}",
                $item_index,
                $item_name,
            ))
            .clone()
            .downcast::<$target_type>()
            .expect(&format!(concat!("PerspectiveController ",
                    "unexpected type for perspective item {:?}",
                ),
                $item_name,
            ))
    };
);

pub struct PerspectiveController {
    menu_button: gtk::MenuButton,
    popover: gtk::PopoverMenu,
    stack: gtk::Stack,
    split_btn: gtk::Button,
}

impl PerspectiveController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        let this_rc = Rc::new(RefCell::new(PerspectiveController {
            menu_button: builder.get_object("perspective-menu-btn").unwrap(),
            popover: builder.get_object("perspective-popovermenu").unwrap(),
            stack: builder.get_object("perspective-stack").unwrap(),
            split_btn: builder.get_object("perspective-split-btn").unwrap(),
        }));

        this_rc.borrow().cleanup();

        this_rc
    }

    pub fn register_callbacks(
        this_rc: &Rc<RefCell<Self>>,
        _main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let this = this_rc.borrow();

        let menu_btn_box = gtk_downcast!(
            this.menu_button
                .get_child()
                .expect("PerspectiveController no box for menu button"),
            gtk::Box,
            "menu button"
        );
        let menu_btn_image = gtk_downcast!(menu_btn_box, 0, gtk::Image, "menu button");
        let popover_box = gtk_downcast!(this.popover, 0, gtk::Box, "popover");

        let mut index = 0;
        let stack_children = this.stack.get_children();
        #[cfg_attr(feature = "cargo-clippy", allow(explicit_counter_loop))]
        for perspective_box_child in popover_box.get_children() {
            let stack_child = stack_children.get(index).expect(&format!(
                "PerspectiveController no stack child for index {:?}",
                index
            ));

            let button = gtk_downcast!(perspective_box_child, gtk::Button, "popover box");
            let button_name = button.get_name();
            let button_box = gtk_downcast!(
                button.get_child().expect(&format!(
                    "PerspectiveController no box for button {:?}",
                    button_name
                )),
                gtk::Box,
                button_name
            );

            let perspective_icon_name = gtk_downcast!(button_box, 0, gtk::Image, button_name)
                .get_property_icon_name()
                .expect(&format!(
                    "PerspectiveController no icon name for button {:?}",
                    button_name,
                ));

            let stack_child_name = this.stack
                .get_child_name(stack_child)
                .expect(&format!(
                    "PerspectiveController no name for stack page matching {:?}",
                    button_name,
                ))
                .to_owned();

            if index == 0 {
                // set the default perspective
                menu_btn_image.set_property_icon_name(Some(&perspective_icon_name));
                this.stack.set_visible_child_name(&stack_child_name);
            }

            let menu_btn_image = menu_btn_image.clone();
            let stack = this.stack.clone();
            let popover = this.popover.clone();
            button.connect_clicked(move |_| {
                menu_btn_image.set_property_icon_name(Some(&perspective_icon_name));
                stack.set_visible_child_name(&stack_child_name);
                // popdown is available from GTK 3.22
                // current package used on package is GTK .18
                popover.hide();
            });

            index += 1;
        }
    }

    pub fn cleanup(&self) {
        self.menu_button.set_sensitive(false);
    }

    pub fn new_media(&self, context: &PlaybackContext) {
        self.menu_button.set_sensitive(true);
        let info = context.info.read().unwrap();
        self.streams_changed(&info);
    }

    pub fn streams_changed(&self, info: &MediaInfo) {
        self.split_btn.set_sensitive(info.streams.is_audio_selected());
    }
}
