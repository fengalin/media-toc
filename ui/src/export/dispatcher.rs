use crate::{
    export,
    generic_output::{self, prelude::*},
    main,
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
    type CtrlImpl = export::ControllerImpl;

    fn ctrl(main_ctrl: &main::Controller) -> &export::Controller {
        &main_ctrl.export
    }

    fn ctrl_mut(main_ctrl: &mut main::Controller) -> &mut export::Controller {
        &mut main_ctrl.export
    }
}
