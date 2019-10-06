use super::MainController;

use super::export_controller::{ExportController, ExportControllerImpl};
use super::output_base_dispatcher::{OutputBaseDispatcher, OutputDispatcherImpl};

pub type ExportDispatcher = OutputBaseDispatcher<ExportDispatcherImpl>;

pub struct ExportDispatcherImpl;
impl OutputDispatcherImpl for ExportDispatcherImpl {
    type CtrlImpl = ExportControllerImpl;

    fn controller(main_ctrl: &MainController) -> &ExportController {
        &main_ctrl.export_ctrl
    }

    fn controller_mut(main_ctrl: &mut MainController) -> &mut ExportController {
        &mut main_ctrl.export_ctrl
    }
}
