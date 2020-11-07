use super::MainController;

use super::output_base_dispatcher::OutputDispatcher;
use super::split_controller::{SplitController, SplitControllerImpl};

pub struct SplitDispatcher;
impl OutputDispatcher for SplitDispatcher {
    type CtrlImpl = SplitControllerImpl;

    fn ctrl(main_ctrl: &MainController) -> &SplitController {
        &main_ctrl.split_ctrl
    }

    fn ctrl_mut(main_ctrl: &mut MainController) -> &mut SplitController {
        &mut main_ctrl.split_ctrl
    }
}
