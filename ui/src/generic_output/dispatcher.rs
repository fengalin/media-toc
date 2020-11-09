use futures::{
    future::{self, LocalBoxFuture},
    prelude::*,
};

use gio::prelude::*;
use gtk::prelude::*;

use log::debug;

use crate::{
    generic_output::{self, prelude::*},
    main,
    prelude::*,
};

pub trait OutputDispatcher {
    type CtrlImpl: OutputControllerImpl + 'static;

    fn ctrl(main_ctrl: &main::Controller) -> &generic_output::Controller<Self::CtrlImpl>;
    fn ctrl_mut(
        main_ctrl: &mut main::Controller,
    ) -> &mut generic_output::Controller<Self::CtrlImpl>;
}

impl<T: OutputDispatcher> UIDispatcher for T {
    type Controller = generic_output::Controller<T::CtrlImpl>;
    type Event = generic_output::Event;

    fn setup(ctrl: &mut Self::Controller, app: &gtk::Application) {
        ctrl.page.connect_map(|_| {
            main::switch_to(T::CtrlImpl::FOCUS_CONTEXT);
        });

        ctrl.btn.connect_clicked(|_| {
            UIEventChannel::send(<T::CtrlImpl as OutputControllerImpl>::OutputEvent::from(
                Self::Event::TriggerAction,
            ));
        });

        ctrl.open_action = Some(
            app.lookup_action("open")
                .unwrap()
                .downcast::<gio::SimpleAction>()
                .unwrap(),
        );
    }

    fn handle_event(
        main_ctrl: &mut main::Controller,
        event: impl Into<Self::Event>,
    ) -> LocalBoxFuture<'_, ()> {
        use generic_output::Event::*;

        let event = event.into();
        debug!("handling {:?}", event);
        match event {
            ActionOver => T::ctrl_mut(main_ctrl).switch_to_available(),
            TriggerAction => {
                if !T::ctrl(main_ctrl).is_busy {
                    if let Some(pipeline) = main_ctrl.pipeline.as_mut() {
                        main_ctrl
                            .info
                            .export_chapters(&mut pipeline.info.write().unwrap());
                        return T::ctrl_mut(main_ctrl).start().boxed_local();
                    }
                } else {
                    T::ctrl_mut(main_ctrl).cancel();
                }
            }
        }

        future::ready(()).boxed_local()
    }
}
