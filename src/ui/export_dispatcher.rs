use super::{
    ExportController, ExportControllerImpl, MainController, OutputBaseDispatcher,
    OutputDispatcherImpl,
};

pub type ExportDispatcher = OutputBaseDispatcher<ExportDispatcherImpl>;

pub struct ExportDispatcherImpl;
impl OutputDispatcherImpl for ExportDispatcherImpl {
    type CtrlImpl = ExportControllerImpl;
    fn controller(main_ctrl: &mut MainController) -> &mut ExportController {
        &mut main_ctrl.export_ctrl
    }
}
