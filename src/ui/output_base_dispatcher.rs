use futures::channel::mpsc as async_mpsc;
use futures::prelude::*;

use gtk;
use gtk::prelude::*;

use log::debug;

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use media::MediaEvent;

use super::{MainController, UIDispatcher, UIEventSender};

use super::output_base_controller::{OutputBaseController, OutputControllerImpl};

pub trait OutputDispatcherImpl {
    type CtrlImpl: OutputControllerImpl;

    fn controller(main_ctrl: &MainController) -> &OutputBaseController<Self::CtrlImpl>;
    fn controller_mut(main_ctrl: &mut MainController) -> &mut OutputBaseController<Self::CtrlImpl>;
}

pub struct OutputBaseDispatcher<Impl> {
    impl_type: PhantomData<Impl>,
}

impl<Impl> UIDispatcher for OutputBaseDispatcher<Impl>
where
    Impl: OutputDispatcherImpl,
    Impl::CtrlImpl: OutputControllerImpl,
{
    type Controller = OutputBaseController<Impl::CtrlImpl>;

    fn setup(
        ctrl: &mut Self::Controller,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        _app: &gtk::Application,
        ui_event: &UIEventSender,
    ) {
        let ui_event = ui_event.clone();
        ctrl.page.connect_map(move |_| {
            ui_event.switch_to(Impl::CtrlImpl::FOCUS_CONTEXT);
        });

        ctrl.btn.connect_clicked(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _| {
                main_ctrl.pause_and_callback(Box::new(|main_ctrl: &mut MainController| {
                    if !Impl::controller_mut(main_ctrl).is_busy {
                        if let Some(pipeline) = main_ctrl.pipeline.as_mut() {
                            main_ctrl.info_ctrl.export_chapters(&mut pipeline.info.write().unwrap());
                            Impl::controller_mut(main_ctrl).start();
                        }
                    } else {
                        Impl::controller_mut(main_ctrl).cancel();
                    }
                }));
            }
        ));

        ctrl.new_media_event_handler = Some(Box::new(
            call_async_with!((main_ctrl_rc) => move async boxed_local |receiver| {
                let mut receiver = receiver;
                while let Some(event) = async_mpsc::Receiver::<MediaEvent>::next(&mut receiver).await {
                    let mut main_ctrl = main_ctrl_rc.borrow_mut();
                    if Impl::controller_mut(&mut main_ctrl).handle_media_event(event).await.is_err() {
                        break;
                    }
                }
                debug!("Output Controller media event handler terminated");
            }),
        ));

        // FIXME use a glib interval
        ctrl.progress_updater = Some(Rc::new(with_main_ctrl!(
            main_ctrl_rc => try move |&mut main_ctrl| {
                Impl::controller_mut(&mut main_ctrl).update_progress()
            } else {
                gtk::Continue(true)
            }
        )));

        ctrl.new_processing_state_handler = Some(Box::new(
            call_async_with!((main_ctrl_rc) => move async boxed_local |state| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                let () = Impl::controller_mut(&mut main_ctrl).handle_processing_states(Ok(state)).await.unwrap();
            }),
        ));
    }
}
