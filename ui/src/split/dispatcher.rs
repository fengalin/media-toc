use crate::{
    generic_output::{self, prelude::*},
    main_panel, split,
};

#[derive(Debug)]
pub struct Event(generic_output::Event);

impl From<Event> for generic_output::Event {
    fn from(event: Event) -> Self {
        event.0
    }
}

impl From<generic_output::Event> for Event {
    fn from(event: generic_output::Event) -> Self {
        Event(event)
    }
}

pub struct Dispatcher;
impl OutputDispatcher for Dispatcher {
    type CtrlImpl = split::ControllerImpl;

    fn ctrl(main_ctrl: &main_panel::Controller) -> &split::Controller {
        &main_ctrl.split
    }

    fn ctrl_mut(main_ctrl: &mut main_panel::Controller) -> &mut split::Controller {
        &mut main_ctrl.split
    }
}
