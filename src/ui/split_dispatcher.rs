use super::{
    MainController, OutputBaseDispatcher, OutputDispatcherImpl, SplitController,
    SplitControllerImpl,
};

pub type SplitDispatcher = OutputBaseDispatcher<SplitDispatcherImpl>;

pub struct SplitDispatcherImpl;
impl OutputDispatcherImpl for SplitDispatcherImpl {
    type CtrlImpl = SplitControllerImpl;

    fn controller(main_ctrl: &MainController) -> &SplitController {
        &main_ctrl.split_ctrl
    }

    fn controller_mut(main_ctrl: &mut MainController) -> &mut SplitController {
        &mut main_ctrl.split_ctrl
    }
}
