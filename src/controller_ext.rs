use std::rc::Rc;
use std::cell::RefCell;
use main_controller::MainController;

pub trait Notifiable {
    fn set_main_controller(&mut self, main_ctrl: Rc<RefCell<MainController>>);
    fn notify_new_media(&mut self);
}
