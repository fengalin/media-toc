use glib;

use gtk;
use gtk::prelude::*;
use log::error;

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use crate::media::MediaEvent;

use super::{
    MainController, MediaProcessor, OutputBaseController, OutputControllerImpl, ProcessingStatus,
    ProcessingType, UIController, UIDispatcher,
};

const PROGRESS_TIMER_PERIOD: u32 = 250; // 250 ms

pub trait OutputDispatcherImpl {
    type CtrlImpl;
    fn controller(main_ctrl: &mut MainController) -> &mut OutputBaseController<Self::CtrlImpl>
    where
        Self::CtrlImpl: MediaProcessor + OutputControllerImpl + UIController + 'static;
}

pub struct OutputBaseDispatcher<Impl> {
    impl_type: PhantomData<Impl>,
}

impl<Impl> OutputBaseDispatcher<Impl>
where
    Impl: OutputDispatcherImpl,
    Impl::CtrlImpl: MediaProcessor + OutputControllerImpl + UIController + 'static,
{
    fn switch_to_busy(main_ctrl: &mut MainController) {
        main_ctrl.set_cursor_waiting();
        Impl::controller(main_ctrl).switch_to_busy();
    }

    fn switch_to_available(main_ctrl: &mut MainController) {
        Impl::controller(main_ctrl).switch_to_available();
        main_ctrl.reset_cursor();
    }

    fn register_media_event_handler(
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        receiver: glib::Receiver<MediaEvent>,
    ) {
        let main_ctrl_rc_clone = Rc::clone(main_ctrl_rc);

        receiver.attach(None, move |event| {
            let mut main_ctrl = main_ctrl_rc_clone.borrow_mut();
            let ctrl = Impl::controller(&mut main_ctrl);

            let is_in_progress = match ctrl.handle_media_event(event) {
                Ok(ProcessingStatus::Completed(msg)) => {
                    main_ctrl.show_info(msg);
                    false
                }
                Ok(ProcessingStatus::InProgress) => true,
                Err(err) => {
                    main_ctrl.show_error(err);
                    false
                }
            };

            if is_in_progress {
                glib::Continue(true)
            } else {
                Self::restore_pipeline(&mut main_ctrl);
                Self::switch_to_available(&mut main_ctrl);
                glib::Continue(false)
            }
        });
    }

    fn register_progress_timer(
        main_ctrl: &mut MainController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
    ) {
        let main_ctrl_rc_clone = Rc::clone(main_ctrl_rc);
        let progress_timer_src = glib::timeout_add_local(PROGRESS_TIMER_PERIOD, move || {
            let mut main_ctrl = main_ctrl_rc_clone.borrow_mut();
            let ctrl = Impl::controller(&mut main_ctrl);

            let progress = ctrl.report_progress();
            ctrl.progress_bar.set_fraction(progress);

            glib::Continue(true)
        });

        Impl::controller(main_ctrl).set_progress_timer_src(progress_timer_src);
    }

    fn restore_pipeline(main_ctrl: &mut MainController) {
        let playback_pipeline = Impl::controller(main_ctrl)
            .playback_pipeline
            .take()
            .unwrap();
        main_ctrl.set_pipeline(playback_pipeline);
    }
}

impl<Impl> UIDispatcher for OutputBaseDispatcher<Impl>
where
    Impl: OutputDispatcherImpl,
    Impl::CtrlImpl: MediaProcessor + OutputControllerImpl + UIController + 'static,
{
    fn setup(_gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>) {
        let mut main_ctrl_local = main_ctrl_rc.borrow_mut();

        let main_ctrl_rc_click = Rc::clone(main_ctrl_rc);
        Impl::controller(&mut main_ctrl_local)
            .btn
            .connect_clicked(move |_| {
                let main_ctrl_rc_req = Rc::clone(&main_ctrl_rc_click);
                main_ctrl_rc_click.borrow_mut().request_pipeline(Box::new(
                    move |main_ctrl, pipeline| {
                        Impl::controller(main_ctrl).playback_pipeline = Some(pipeline);

                        Self::switch_to_busy(main_ctrl);

                        match Impl::controller(main_ctrl).init() {
                            ProcessingType::Sync => (),
                            ProcessingType::Async(receiver) => {
                                Self::register_media_event_handler(&main_ctrl_rc_req, receiver);
                                Self::register_progress_timer(main_ctrl, &main_ctrl_rc_req);
                            }
                        }

                        let is_in_progress = match Impl::controller(main_ctrl).start() {
                            Ok(ProcessingStatus::Completed(msg)) => {
                                main_ctrl.show_info(msg);
                                false
                            }
                            Ok(ProcessingStatus::InProgress) => true,
                            Err(err) => {
                                error!("{}", err);
                                main_ctrl.show_error(err);
                                false
                            }
                        };

                        if !is_in_progress {
                            Self::restore_pipeline(main_ctrl);
                            Self::switch_to_available(main_ctrl);
                        }
                    },
                ));
            });
    }
}
