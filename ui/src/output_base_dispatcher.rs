use gio::prelude::*;
use glib::clone;
use gtk::prelude::*;

use super::{MainController, UIDispatcher, UIEventSender};

use super::output_base_controller::{OutputBaseController, OutputControllerImpl};

pub trait OutputDispatcher {
    type CtrlImpl: OutputControllerImpl + 'static;

    fn ctrl(main_ctrl: &MainController) -> &OutputBaseController<Self::CtrlImpl>;
    fn ctrl_mut(main_ctrl: &mut MainController) -> &mut OutputBaseController<Self::CtrlImpl>;
}

impl<T: OutputDispatcher> UIDispatcher for T {
    type Controller = OutputBaseController<T::CtrlImpl>;

    fn setup(ctrl: &mut Self::Controller, app: &gtk::Application, ui_event: &UIEventSender) {
        ctrl.page.connect_map(clone!(@strong ui_event => move |_| {
            ui_event.switch_to(T::CtrlImpl::FOCUS_CONTEXT);
        }));

        ctrl.btn
            .connect_clicked(clone!(@strong ui_event => move |_| {
                ui_event.trigger_action(T::CtrlImpl::FOCUS_CONTEXT);
            }));

        ctrl.open_action = Some(
            app.lookup_action("open")
                .unwrap()
                .downcast::<gio::SimpleAction>()
                .unwrap(),
        );
    }
}
