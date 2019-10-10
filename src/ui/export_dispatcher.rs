use std::{
    cell::{Ref, RefCell, RefMut},
    rc::Rc,
};

use super::MainController;

use super::export_controller::{ExportController, ExportControllerImpl};
use super::output_base_dispatcher::{OutputBaseDispatcher, OutputDispatcherImpl};

pub type ExportDispatcher = OutputBaseDispatcher<ExportDispatcherImpl>;

pub struct ExportDispatcherImpl;
impl OutputDispatcherImpl for ExportDispatcherImpl {
    type CtrlImpl = ExportControllerImpl;

    fn ctrl(main_ctrl: &MainController) -> &ExportController {
        &main_ctrl.export_ctrl
    }

    fn ctrl_ref(main_ctrl: &Rc<RefCell<MainController>>) -> Ref<'_, ExportController> {
        Ref::map(main_ctrl.borrow(), |main_ctrl| &main_ctrl.export_ctrl)
    }

    fn ctrl_mut(main_ctrl: &mut MainController) -> &mut ExportController {
        &mut main_ctrl.export_ctrl
    }

    fn ctrl_ref_mut(main_ctrl: &Rc<RefCell<MainController>>) -> RefMut<'_, ExportController> {
        RefMut::map(main_ctrl.borrow_mut(), |main_ctrl| {
            &mut main_ctrl.export_ctrl
        })
    }
}
