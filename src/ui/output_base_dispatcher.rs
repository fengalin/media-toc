use glib;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use super::{
    MainController, MediaProcessor, OutputBaseController, OutputControllerImpl, ProcessingState,
    UIController, UIDispatcher,
};

pub trait OutputDispatcherImpl {
    type CtrlImpl;
    fn controller(main_ctrl: &mut MainController) -> &mut OutputBaseController<Self::CtrlImpl>
    where
        Self::CtrlImpl: MediaProcessor + OutputControllerImpl + UIController + 'static;
}

macro_rules! new_dispatcher {
    (@param _) => ( _ );
    (@param $x:ident) => ( $x );
    ($($n:ident),+ => move || $body:expr) => (
        {
            $( let $n = $n.clone(); )+
            Some(Box::new(move || {
                $( let $n = $n.clone(); )+
                gtk::idle_add(move || {
                    $body;
                    glib::Continue(false)
                });
            }))
        }
    );
    ($($n:ident),+ => move |$($p:tt),+| $body:expr) => (
        {
            $( let $n = $n.clone(); )+
            Some(Box::new(move |$(new_dispatcher!(@param $p),)+| {
                $( let $n = $n.clone(); )+
                gtk::idle_add(move || {
                    $body;
                    glib::Continue(false)
                });
            }))
        }
    );
}

pub struct OutputBaseDispatcher<Impl> {
    impl_type: PhantomData<Impl>,
}

impl<Impl> UIDispatcher for OutputBaseDispatcher<Impl>
where
    Impl: OutputDispatcherImpl,
    Impl::CtrlImpl: MediaProcessor + OutputControllerImpl + UIController + 'static,
{
    fn setup(_gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>) {
        let mut main_ctrl_local = main_ctrl_rc.borrow_mut();
        let ctrl = Impl::controller(&mut main_ctrl_local);

        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        ctrl.btn.connect_clicked(move |_| {
            main_ctrl_rc_cb
                .borrow_mut()
                .request_pipeline(Box::new(move |main_ctrl, pipeline| {
                    let ctrl = Impl::controller(main_ctrl);
                    ctrl.playback_pipeline = Some(pipeline);
                    ctrl.handle_processing_states(Ok(ProcessingState::Start));
                }));
        });

        let main_ctrl_rc_hdlr = Rc::clone(main_ctrl_rc);
        ctrl.handle_media_event_async = Some(Rc::new(move |event| {
            let mut main_ctrl = main_ctrl_rc_hdlr.borrow_mut();
            let ctrl = Impl::controller(&mut main_ctrl);
            let res = ctrl.handle_media_event(event);
            ctrl.handle_processing_states(res);
            // will be removed in `OutputBaseController::switch_to_available`
            glib::Continue(true)
        }));

        let main_ctrl_rc_fn = Rc::clone(main_ctrl_rc);
        ctrl.progress_updater = Some(Rc::new(move || {
            let mut main_ctrl = main_ctrl_rc_fn.borrow_mut();
            let ctrl = Impl::controller(&mut main_ctrl);

            if let Some(progress) = ctrl.report_progress() {
                ctrl.progress_bar.set_fraction(progress);
            }

            glib::Continue(true)
        }));

        ctrl.cursor_waiting_dispatcher = new_dispatcher!(
            main_ctrl_rc => move || {
                main_ctrl_rc.borrow().set_cursor_waiting();
            }
        );

        ctrl.hand_back_to_main_ctrl_dispatcher = new_dispatcher!(
            main_ctrl_rc => move || {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                let playback_pipeline = Impl::controller(&mut main_ctrl)
                    .playback_pipeline
                    .take()
                    .expect(concat!(
                        "OutputBaseDispatcher: no `playback_pipeline` in ",
                        "`hand_back_to_main_ctrl_dispatcher`",
                    ));
                main_ctrl.set_pipeline(playback_pipeline);
                main_ctrl.reset_cursor();
            }
        );

        ctrl.overwrite_question_dispatcher = new_dispatcher!(
            main_ctrl_rc => move |question, path| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.reset_cursor();
                let path_resp = Rc::clone(&path);
                let main_ctrl_resp = Rc::clone(&main_ctrl_rc);
                main_ctrl.show_question(
                    &question,
                    Box::new(move |response_type| {
                        let mut main_ctrl = main_ctrl_resp.borrow_mut();
                        Impl::controller(&mut main_ctrl).handle_processing_states(Ok(
                            ProcessingState::GotUserResponse(response_type, Rc::clone(&path_resp)),
                        ));
                    }),
                );
            }
        );

        ctrl.show_error_dispatcher = new_dispatcher!(
            main_ctrl_rc => move |msg| {
                main_ctrl_rc.borrow_mut().show_error(msg.as_ref());
            }
        );

        ctrl.show_info_dispatcher = new_dispatcher!(
            main_ctrl_rc => move |msg| {
                main_ctrl_rc.borrow_mut().show_info(msg.as_ref());
            }
        );
    }
}
