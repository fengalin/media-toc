use gettextrs::gettext;
use glib;
use gtk;
use gtk::prelude::*;
use log::error;

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use crate::media::MediaEvent;

use super::{
    MainController, MediaProcessor, OutputBaseController, OutputControllerImpl, ProcessingState,
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

    fn handle_processing_states(
        main_ctrl: &mut MainController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        mut res: Result<ProcessingState, String>,
    ) {
        loop {
            match res {
                Ok(ProcessingState::Cancelled) => {
                    Self::restore_pipeline(main_ctrl);
                    Self::switch_to_available(main_ctrl);
                    main_ctrl.show_info(gettext("Operation cancelled"));
                    break;
                }
                Ok(ProcessingState::ConfirmedOutputTo(path)) => {
                    let ctrl = Impl::controller(main_ctrl);
                    res = match ctrl.process(path.as_ref()) {
                        Ok(()) => {
                            if ctrl.media_event_async_handler_src.is_some() {
                                // Don't handle `next()` locally if processing asynchronously
                                // Next steps handled asynchronously (media event handler)
                                break;
                            } else {
                                // processing synchronously
                                Ok(ProcessingState::DoneWithCurrent)
                            }
                        }
                        Err(err) => Err(err),
                    };
                }
                Ok(ProcessingState::CurrentSkipped) => {
                    res = match Impl::controller(main_ctrl).next() {
                        Ok(state) => match state {
                            ProcessingState::AllComplete(_) => {
                                // Don't display the success message when the user decided
                                // to skip (not overwrite) last part as it seems missleading
                                Self::restore_pipeline(main_ctrl);
                                Self::switch_to_available(main_ctrl);
                                break;
                            }
                            other => Ok(other),
                        },
                        Err(err) => Err(err),
                    };
                }
                Ok(ProcessingState::DoneWithCurrent) => {
                    res = Impl::controller(main_ctrl).next();
                }
                Ok(ProcessingState::InProgress) => {
                    // Next steps handled asynchronously (media event handler)
                    break;
                }
                Ok(ProcessingState::AllComplete(msg)) => {
                    Self::restore_pipeline(main_ctrl);
                    Self::switch_to_available(main_ctrl);
                    main_ctrl.show_info(msg);
                    break;
                }
                Ok(ProcessingState::WouldOutputTo(path)) => {
                    if path.exists() {
                        main_ctrl.reset_cursor();
                        let main_ctrl_cb = Rc::clone(main_ctrl_rc);
                        main_ctrl.show_question(
                            gettext("{}\nAlready exists. Overwrite?").replacen(
                                "{}",
                                path.to_str().as_ref().unwrap(),
                                1,
                            ),
                            Box::new(move |response_type| {
                                let mut main_ctrl = main_ctrl_cb.borrow_mut();
                                let next_state = match response_type {
                                    gtk::ResponseType::Yes => {
                                        ProcessingState::ConfirmedOutputTo(path.clone())
                                    }
                                    gtk::ResponseType::No => ProcessingState::CurrentSkipped,
                                    gtk::ResponseType::Cancel => ProcessingState::Cancelled,
                                    other => unreachable!(
                                        concat!(
                                            "Response type {:?} in ",
                                            "OutputBaseDispatcher::handle_processing_states",
                                        ),
                                        other,
                                    ),
                                };
                                Self::handle_processing_states(
                                    &mut main_ctrl,
                                    &main_ctrl_cb,
                                    Ok(next_state),
                                );
                            }),
                        );

                        // Pending user confirmation
                        // Next steps handled asynchronously (see closure above)
                        break;
                    } else {
                        // handle processing in next iteration
                        res = Ok(ProcessingState::ConfirmedOutputTo(path));
                    }
                }
                Err(err) => {
                    error!("{}", err);
                    Self::restore_pipeline(main_ctrl);
                    Self::switch_to_available(main_ctrl);
                    main_ctrl.show_error(&err);
                    break;
                }
            }
        }
    }

    fn attach_media_event_async_handler(
        main_ctrl: &mut MainController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        receiver: glib::Receiver<MediaEvent>,
    ) {
        let main_ctrl_rc_clone = Rc::clone(main_ctrl_rc);

        Impl::controller(main_ctrl).media_event_async_handler_src =
            Some(receiver.attach(None, move |event| {
                let mut main_ctrl = main_ctrl_rc_clone.borrow_mut();
                let res = Impl::controller(&mut main_ctrl).handle_media_event(event);
                Self::handle_processing_states(&mut main_ctrl, &main_ctrl_rc_clone, res);
                // will be removed in `OutputBaseController::switch_to_available`
                glib::Continue(true)
            }));
    }

    fn register_progress_timer(
        main_ctrl: &mut MainController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
    ) {
        let main_ctrl_rc_clone = Rc::clone(main_ctrl_rc);
        let progress_timer_src = glib::timeout_add_local(PROGRESS_TIMER_PERIOD, move || {
            let mut main_ctrl = main_ctrl_rc_clone.borrow_mut();
            let ctrl = Impl::controller(&mut main_ctrl);

            if let Some(progress) = ctrl.report_progress() {
                ctrl.progress_bar.set_fraction(progress);
            }

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

        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        Impl::controller(&mut main_ctrl_local)
            .btn
            .connect_clicked(move |_| {
                let main_ctrl_rc_fn = Rc::clone(&main_ctrl_rc_cb);
                main_ctrl_rc_cb.borrow_mut().request_pipeline(Box::new(
                    move |main_ctrl, pipeline| {
                        Impl::controller(main_ctrl).playback_pipeline = Some(pipeline);

                        Self::switch_to_busy(main_ctrl);

                        match Impl::controller(main_ctrl).init() {
                            ProcessingType::Sync => (),
                            ProcessingType::Async(receiver) => {
                                Self::attach_media_event_async_handler(
                                    main_ctrl,
                                    &main_ctrl_rc_fn,
                                    receiver,
                                );
                                Self::register_progress_timer(main_ctrl, &main_ctrl_rc_fn);
                            }
                        }

                        let res = Impl::controller(main_ctrl).next();
                        Self::handle_processing_states(main_ctrl, &main_ctrl_rc_fn, res);
                    },
                ));
            });
    }
}
