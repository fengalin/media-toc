use gtk::{gio, glib::Cast, prelude::*};

use crate::{perspective, prelude::*};

macro_rules! gtk_downcast(
    ($source:expr, $target_type:ty, $item_name:expr) => {
        $source.clone()
            .downcast::<$target_type>()
            .unwrap_or_else(|_| panic!("Unexpected type for perspective item {:?}", $item_name))
    };
    ($source:expr, $item_index:expr, $target_type:ty, $item_name:expr) => {
        $source.children()
            .get($item_index)
            .expect(&format!("PerspectiveController no child at index {} for perspective item {:?}",
                $item_index,
                $item_name,
            ))
            .clone()
            .downcast::<$target_type>()
            .unwrap_or_else(|_| panic!("Unexpected type for perspective item {:?}", $item_name))
    };
);

pub struct Dispatcher;
impl UIDispatcher for Dispatcher {
    type Controller = perspective::Controller;
    type Event = ();

    fn setup(perspective_ctrl: &mut perspective::Controller, app: &gtk::Application) {
        let menu_btn_box = gtk_downcast!(
            perspective_ctrl
                .menu_btn
                .child()
                .expect("perspective::Controller no box for menu button"),
            gtk::Box,
            "menu button"
        );
        let menu_btn_image = gtk_downcast!(menu_btn_box, 0, gtk::Image, "menu button");

        let popover_box = gtk_downcast!(perspective_ctrl.popover, 0, gtk::Box, "popover");

        let stack_children = perspective_ctrl.stack.children();
        for (index, perspective_box_child) in popover_box.children().iter().enumerate() {
            let stack_child = stack_children.get(index).unwrap_or_else(|| {
                panic!(
                    "perspective::Controller no stack child for index {:?}",
                    index
                )
            });

            let button = gtk_downcast!(perspective_box_child, gtk::Button, "popover box");
            let button_name = button.widget_name();
            let button_box = gtk_downcast!(
                button.child().unwrap_or_else(|| panic!(
                    "perspective::Controller no box for button {:?}",
                    button_name
                )),
                gtk::Box,
                button_name
            );

            let perspective_icon_name = gtk_downcast!(button_box, 0, gtk::Image, button_name)
                .icon_name()
                .unwrap_or_else(|| {
                    panic!(
                        "perspective::Controller no icon name for button {:?}",
                        button_name,
                    )
                });

            let stack_child_name = perspective_ctrl
                .stack
                .child_name(stack_child)
                .unwrap_or_else(|| {
                    panic!(
                        "perspective::Controller no name for stack page matching {:?}",
                        button_name,
                    )
                })
                .to_owned();

            if index == 0 {
                // set the default perspective
                menu_btn_image.set_icon_name(Some(perspective_icon_name.as_str()));
                perspective_ctrl
                    .stack
                    .set_visible_child_name(&stack_child_name);
            }

            button.set_sensitive(true);

            let menu_btn_image = menu_btn_image.clone();
            let stack_clone = perspective_ctrl.stack.clone();
            let popover_clone = perspective_ctrl.popover.clone();
            let event = move || {
                menu_btn_image.set_icon_name(Some(perspective_icon_name.as_str()));
                stack_clone.set_visible_child_name(&stack_child_name);
                // popdown is available from GTK 3.22
                // current package used on travis is GTK 3.18
                popover_clone.hide();
            };

            match button.action_name() {
                Some(action_name) => {
                    let accel_key = gtk_downcast!(button_box, 2, gtk::Label, button_name).text();
                    let action_splits: Vec<&str> = action_name.splitn(2, '.').collect();
                    if action_splits.len() != 2 {
                        panic!(
                            "perspective::Controller unexpected action name for button {:?}",
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
        }
    }
}
