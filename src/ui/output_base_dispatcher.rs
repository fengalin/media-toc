use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use crate::with_main_ctrl;

use super::{MainController, OutputBaseController, OutputControllerImpl, UIDispatcher};

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
    ) {
        ctrl.btn.connect_map(move |btn| {
            btn.grab_default();
        });
        ctrl.list.connect_map(move |list| {
            if let Some(selected_row) = list.get_selected_row() {
                selected_row.grab_focus();
            }
        });

        ctrl.btn.connect_clicked(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _| {
                main_ctrl.pause_and_callback(Box::new(|main_ctrl: &mut MainController| {
                    if !Impl::controller_mut(main_ctrl).is_busy() {
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
