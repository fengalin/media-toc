use std::rc::Rc;
use std::cell::RefCell;

use ::media::Context;

use super::MainController;

pub trait Notifiable {
    fn set_main_controller(&mut self, main_ctrl: Rc<RefCell<MainController>>);
    fn notify_new_media(&mut self, &mut Context);
}
