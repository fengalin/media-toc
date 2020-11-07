use super::MainController;

use super::export_controller::{ExportController, ExportControllerImpl};
use super::output_base_dispatcher::OutputDispatcher;

pub struct ExportDispatcher;
impl OutputDispatcher for ExportDispatcher {
    type CtrlImpl = ExportControllerImpl;

    fn ctrl(main_ctrl: &MainController) -> &ExportController {
        &main_ctrl.export_ctrl
    }

    fn ctrl_mut(main_ctrl: &mut MainController) -> &mut ExportController {
        &mut main_ctrl.export_ctrl
    }
}
