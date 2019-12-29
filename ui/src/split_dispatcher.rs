use std::{
    cell::{Ref, RefCell, RefMut},
    rc::Rc,
};

use super::MainController;

use super::output_base_dispatcher::{OutputBaseDispatcher, OutputDispatcherImpl};
use super::split_controller::{SplitController, SplitControllerImpl};

pub type SplitDispatcher = OutputBaseDispatcher<SplitDispatcherImpl>;

pub struct SplitDispatcherImpl;
impl OutputDispatcherImpl for SplitDispatcherImpl {
    type CtrlImpl = SplitControllerImpl;

    fn ctrl(main_ctrl: &MainController) -> &SplitController {
        &main_ctrl.split_ctrl
    }

    fn ctrl_ref(main_ctrl: &Rc<RefCell<MainController>>) -> Ref<'_, SplitController> {
        Ref::map(main_ctrl.borrow(), |main_ctrl| &main_ctrl.split_ctrl)
    }

    fn ctrl_mut(main_ctrl: &mut MainController) -> &mut SplitController {
        &mut main_ctrl.split_ctrl
    }

    fn ctrl_ref_mut(main_ctrl: &Rc<RefCell<MainController>>) -> RefMut<'_, SplitController> {
        RefMut::map(main_ctrl.borrow_mut(), |main_ctrl| {
            &mut main_ctrl.split_ctrl
        })
    }
}
