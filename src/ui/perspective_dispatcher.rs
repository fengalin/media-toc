use gio;
use gio::prelude::*;
use glib::Cast;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use super::{MainController, PerspectiveController, UIDispatcher, UIEventSender};

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

pub struct PerspectiveDispatcher;
impl UIDispatcher for PerspectiveDispatcher {
    type Controller = PerspectiveController;

    fn setup(
        perspective_ctrl: &mut PerspectiveController,
        _main_ctrl_rc: &Rc<RefCell<MainController>>,
        app: &gtk::Application,
        _ui_event_sender: &UIEventSender,
    ) {
        let menu_btn_box = gtk_downcast!(
            perspective_ctrl
                .menu_btn
                .get_child()
                .expect("PerspectiveController no box for menu button"),
            gtk::Box,
            "menu button"
        );
        let menu_btn_image = gtk_downcast!(menu_btn_box, 0, gtk::Image, "menu button");

        let popover_box = gtk_downcast!(perspective_ctrl.popover, 0, gtk::Box, "popover");

        let mut index = 0;
        let stack_children = perspective_ctrl.stack.get_children();
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

            let stack_child_name = perspective_ctrl
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
                perspective_ctrl
                    .stack
                    .set_visible_child_name(&stack_child_name);
            }

            button.set_sensitive(true);

            let menu_btn_image = menu_btn_image.clone();
            let stack_clone = perspective_ctrl.stack.clone();
            let popover_clone = perspective_ctrl.popover.clone();
            let event = move || {
                menu_btn_image.set_property_icon_name(Some(perspective_icon_name.as_str()));
                stack_clone.set_visible_child_name(&stack_child_name);
                // popdown is available from GTK 3.22
                // current package used on travis is GTK 3.18
                popover_clone.hide();
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
                    app.add_action(&action);
                    action.connect_activate(move |_, _| event());
                    app.set_accels_for_action(&action_name, &[&accel_key]);
                }
                None => {
                    button.connect_clicked(move |_| event());
                }
            }

            index += 1;
        }
    }
}
