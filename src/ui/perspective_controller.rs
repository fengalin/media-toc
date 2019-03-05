use gio;
use gio::prelude::*;
use glib::Cast;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use crate::{media::PlaybackPipeline, metadata::MediaInfo};

use super::{MainController, UIController};

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
    pub fn new_rc(builder: &gtk::Builder) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(PerspectiveController {
            menu_button: builder.get_object("perspective-menu-btn").unwrap(),
            popover: builder.get_object("perspective-popovermenu").unwrap(),
            stack: builder.get_object("perspective-stack").unwrap(),
            split_btn: builder.get_object("perspective-split-btn").unwrap(),
        }))
    }
}

impl UIController for PerspectiveController {
    fn setup_(
        this_rc: &Rc<RefCell<Self>>,
        gtk_app: &gtk::Application,
        _main_ctrl: &Rc<RefCell<MainController>>,
    ) {
        let mut this = this_rc.borrow_mut();

        this.cleanup();

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
        for perspective_box_child in popover_box.get_children() {
            let stack_child = stack_children.get(index).unwrap_or_else(|| {
                panic!("PerspectiveController no stack child for index {:?}", index)
            });

            let button = gtk_downcast!(perspective_box_child, gtk::Button, "popover box");
            let button_name = gtk::WidgetExt::get_name(&button);
            let button_box = gtk_downcast!(
                button.get_child().unwrap_or_else(|| panic!(
                    "PerspectiveController no box for button {:?}",
                    button_name
                )),
                gtk::Box,
                button_name
            );

            let perspective_icon_name = gtk_downcast!(button_box, 0, gtk::Image, button_name)
                .get_property_icon_name()
                .unwrap_or_else(|| {
                    panic!(
                        "PerspectiveController no icon name for button {:?}",
                        button_name,
                    )
                });

            let stack_child_name = this
                .stack
                .get_child_name(stack_child)
                .unwrap_or_else(|| {
                    panic!(
                        "PerspectiveController no name for stack page matching {:?}",
                        button_name,
                    )
                })
                .to_owned();

            if index == 0 {
                // set the default perspective
                menu_btn_image.set_property_icon_name(Some(perspective_icon_name.as_str()));
                this.stack.set_visible_child_name(&stack_child_name);
            }

            button.set_sensitive(true);

            let menu_btn_image = menu_btn_image.clone();
            let stack = this.stack.clone();
            let popover = this.popover.clone();
            let event = move || {
                menu_btn_image.set_property_icon_name(Some(perspective_icon_name.as_str()));
                stack.set_visible_child_name(&stack_child_name);
                // popdown is available from GTK 3.22
                // current package used on package is GTK .18
                popover.hide();
            };

            match button.get_action_name() {
                Some(action_name) => {
                    let accel_key = gtk_downcast!(button_box, 2, gtk::Label, button_name)
                        .get_text()
                        .unwrap_or_else(|| {
                            panic!(
                                "PerspectiveController no acceleration label for button {:?}",
                                button_name,
                            )
                        });
                    let action_splits: Vec<&str> = action_name.splitn(2, '.').collect();
                    if action_splits.len() != 2 {
                        panic!(
                            "PerspectiveController unexpected action name for button {:?}",
                            button_name,
                        );
                    }

                    let action = gio::SimpleAction::new(action_splits[1], None);
                    gtk_app.add_action(&action);
                    action.connect_activate(move |_, _| event());
                    gtk_app.set_accels_for_action(&action_name, &[&accel_key]);
                }
                None => {
                    button.connect_clicked(move |_| event());
                }
            }

            index += 1;
        }
    }

    fn new_media(&mut self, pipeline: &PlaybackPipeline) {
        self.menu_button.set_sensitive(true);
        let info = pipeline.info.read().unwrap();
        self.streams_changed(&info);
    }

    fn cleanup(&mut self) {
        self.menu_button.set_sensitive(false);
    }

    fn streams_changed(&mut self, info: &MediaInfo) {
        self.split_btn
            .set_sensitive(info.streams.is_audio_selected());
    }
}
