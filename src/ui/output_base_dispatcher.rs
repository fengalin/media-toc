use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use crate::with_main_ctrl;

use super::{
    MainController, MediaProcessor, OutputBaseController, OutputControllerImpl, ProcessingState,
    UIController, UIDispatcher,
};

pub trait OutputDispatcherImpl {
    type CtrlImpl;

    fn controller(main_ctrl: &MainController) -> &OutputBaseController<Self::CtrlImpl>
    where
        Self::CtrlImpl: MediaProcessor + OutputControllerImpl + UIController + 'static;

    fn controller_mut(main_ctrl: &mut MainController) -> &mut OutputBaseController<Self::CtrlImpl>
    where
        Self::CtrlImpl: MediaProcessor + OutputControllerImpl + UIController + 'static;
}

pub struct OutputBaseDispatcher<Impl> {
    impl_type: PhantomData<Impl>,
}

impl<Impl> UIDispatcher for OutputBaseDispatcher<Impl>
where
    Impl: OutputDispatcherImpl,
    Impl::CtrlImpl: MediaProcessor + OutputControllerImpl + UIController + 'static,
{
    fn setup(_gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>) {
        let mut main_ctrl = main_ctrl_rc.borrow_mut();
        let ctrl = Impl::controller_mut(&mut main_ctrl);

        ctrl.btn.connect_clicked(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _| {
                main_ctrl.pause_and_callback(Box::new(|main_ctrl: &mut MainController| {
                    if let Some(pipeline) = main_ctrl.pipeline.as_mut() {
                        main_ctrl.info_ctrl.export_chapters(&mut pipeline.info.write().unwrap());
                        Impl::controller_mut(main_ctrl)
                            .handle_processing_states(Ok(ProcessingState::Start));
                    }
                }));
            }
        ));

        ctrl.media_event_handler = Some(Rc::new(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, event| {
                Impl::controller_mut(&mut main_ctrl).handle_media_event(event);
            }
        )));

        ctrl.progress_updater = Some(Rc::new(with_main_ctrl!(
            main_ctrl_rc => move |&main_ctrl| {
                Impl::controller(&main_ctrl).update_progress();
            }
        )));

        ctrl.overwrite_response_cb = Some(Rc::new(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, response_type, path| {
                Impl::controller_mut(&mut main_ctrl).handle_overwrite_response(response_type, path);
            }
        )));
    }
}
