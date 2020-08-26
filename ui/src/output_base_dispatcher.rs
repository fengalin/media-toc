use futures::channel::mpsc as async_mpsc;
use futures::prelude::*;

use gettextrs::gettext;
use glib::clone;
use gtk::prelude::*;

use gio::prelude::*;

use log::debug;

use std::{
    cell::{Ref, RefCell, RefMut},
    marker::PhantomData,
    rc::Rc,
};

use media::MediaEvent;

use super::{MainController, UIDispatcher, UIEventSender};

use super::output_base_controller::{
    MediaProcessor, OutputBaseController, OutputControllerImpl, ProcessingState,
};

const PROGRESS_TIMER_PERIOD: u32 = 250; // 250 ms

pub trait OutputDispatcherImpl {
    type CtrlImpl: OutputControllerImpl;

    fn ctrl(main_ctrl: &MainController) -> &OutputBaseController<Self::CtrlImpl>;
    fn ctrl_ref(
        main_ctrl: &Rc<RefCell<MainController>>,
    ) -> Ref<'_, OutputBaseController<Self::CtrlImpl>>;
    fn ctrl_mut(main_ctrl: &mut MainController) -> &mut OutputBaseController<Self::CtrlImpl>;
    fn ctrl_ref_mut(
        main_ctrl: &Rc<RefCell<MainController>>,
    ) -> RefMut<'_, OutputBaseController<Self::CtrlImpl>>;
}

pub struct OutputBaseDispatcher<Impl> {
    impl_type: PhantomData<Impl>,
}

impl<Impl> UIDispatcher for OutputBaseDispatcher<Impl>
where
    Impl: OutputDispatcherImpl,
    Impl::CtrlImpl: OutputControllerImpl,
{
    type Controller = OutputBaseController<Impl::CtrlImpl>;

    fn setup(
        ctrl: &mut Self::Controller,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        app: &gtk::Application,
        ui_event: &UIEventSender,
    ) {
        let ui_event_clone = ui_event.clone();
        ctrl.page.connect_map(move |_| {
            ui_event_clone.switch_to(Impl::CtrlImpl::FOCUS_CONTEXT);
        });

        ctrl.btn.connect_clicked(clone!(@weak main_ctrl_rc => move |_| {
            let mut main_ctrl = main_ctrl_rc.borrow_mut();
            main_ctrl.pause_and_callback(Box::new(|main_ctrl: &mut MainController| {
                if !Impl::ctrl_mut(main_ctrl).is_busy {
                    if let Some(pipeline) = main_ctrl.pipeline.as_mut() {
                        main_ctrl.info_ctrl.export_chapters(&mut pipeline.info.write().unwrap());
                        Impl::ctrl_mut(main_ctrl).start();
                    }
                } else {
                    Impl::ctrl_mut(main_ctrl).cancel();
                }
            }));
        }));

        ctrl.new_media_event_handler = Some(Box::new(
            clone!(@weak main_ctrl_rc => @default-panic, move |receiver| {
                let main_ctrl_rc = Rc::clone(&main_ctrl_rc);
                async move {
                    let mut receiver = receiver;
                    while let Some(event) = async_mpsc::Receiver::<MediaEvent>::next(&mut receiver).await {
                        debug!("handling media event {:?}", event);
                        let processing_state_handler = {
                            let mut ctrl = Impl::ctrl_ref_mut(&main_ctrl_rc);
                            let res = ctrl.impl_.handle_media_event(event);
                            ctrl.new_processing_state_handler.as_ref().unwrap()(res)
                        };

                        if processing_state_handler.await.is_err() {
                            break;
                        }
                    }
                    debug!("media event handler terminated");
                }.boxed_local()
            }),
        ));

        ctrl.new_progress_updater = Some(Box::new(
            clone!(@weak main_ctrl_rc => @default-panic, move || {
                let main_ctrl_rc = Rc::clone(&main_ctrl_rc);
                async move {
                    let mut stream = glib::interval_stream(PROGRESS_TIMER_PERIOD);
                    loop {
                        let _ = stream.next().await;
                        if Impl::ctrl_ref_mut(&main_ctrl_rc).update_progress().is_err() {
                            break;
                        }
                    }
                }.boxed_local()
            }),
        ));

        let btn = ctrl.btn.clone();
        ctrl.new_processing_state_handler = Some(Box::new(clone!(
            @weak main_ctrl_rc, @strong ui_event => @default-panic, move |state| {
                let main_ctrl_rc = Rc::clone(&main_ctrl_rc);
                let ui_event = ui_event.clone();
                let btn = btn.clone();
                async move {
                    let mut state = state;
                    let res = loop {
                        debug!("handling processing state {:?}", state);

                        match state {
                            Ok(ProcessingState::AllComplete(msg)) => {
                                ui_event.show_info(msg);
                                break Err(());
                            }
                            Ok(ProcessingState::ConfirmedOutputTo(path)) => {
                                state = Impl::ctrl_ref_mut(&main_ctrl_rc).impl_.process(path.as_ref());
                                if state == Ok(ProcessingState::PendingAsyncMediaEvent) {
                                    // Next state handled asynchronously in media event handler
                                    break Ok(());
                                }
                            }
                            Ok(ProcessingState::DoneWithCurrent) => {
                                state = Impl::ctrl_ref_mut(&main_ctrl_rc).impl_.next();
                            }
                            Ok(ProcessingState::PendingAsyncMediaEvent) => {
                                // Next state handled asynchronously in media event handler
                                break Ok(());
                            }
                            Ok(ProcessingState::Start) => {
                                state = Impl::ctrl_ref_mut(&main_ctrl_rc).impl_.next();
                            }
                            Ok(ProcessingState::SkipCurrent) => {
                                state = match Impl::ctrl_ref_mut(&main_ctrl_rc).impl_.next() {
                                    Ok(state) => match state {
                                        ProcessingState::AllComplete(_) => {
                                            // Don't display the success message when the user decided
                                            // to skip (not overwrite) last part as it seems missleading
                                            break Err(());
                                        }
                                        other => Ok(other),
                                    },
                                    Err(err) => Err(err),
                                };
                            }
                            Ok(ProcessingState::WouldOutputTo(path)) => {
                                if path.exists() {
                                    if Impl::ctrl_ref_mut(&main_ctrl_rc).overwrite_all {
                                        state = Ok(ProcessingState::ConfirmedOutputTo(path));
                                        continue;
                                    }
                                } else {
                                    state = Ok(ProcessingState::ConfirmedOutputTo(path));
                                    continue;
                                }

                                // Path exists and overwrite_all is not true
                                btn.set_sensitive(false);
                                ui_event.reset_cursor();

                                let filename = path.file_name().expect("no `filename` in `path`");
                                let filename = filename
                                    .to_str()
                                    .expect("can't get printable `str` from `filename`");
                                let question = gettext("{output_file}\nalready exists. Overwrite?").replacen(
                                    "{output_file}",
                                    filename,
                                    1,
                                );

                                let response = ui_event.ask_question(question).await;
                                btn.set_sensitive(true);

                                let mut ctrl = Impl::ctrl_ref_mut(&main_ctrl_rc);
                                let next_state = match response {
                                    gtk::ResponseType::Apply => {
                                        // This one is used for "Yes to all"
                                        ctrl.overwrite_all = true;
                                        ProcessingState::ConfirmedOutputTo(Rc::clone(&path))
                                    }
                                    gtk::ResponseType::Cancel => {
                                        ctrl.cancel();
                                        break Err(());
                                    }
                                    gtk::ResponseType::No => ProcessingState::SkipCurrent,
                                    gtk::ResponseType::Yes => {
                                        ProcessingState::ConfirmedOutputTo(Rc::clone(&path))
                                    }
                                    other => unimplemented!(
                                        "Response {:?} in OutputBaseController::ask_overwrite_question",
                                        other,
                                    ),
                                };

                                ui_event.set_cursor_waiting();
                                state = Ok(next_state);
                            }
                            Err(err) => {
                                ui_event.show_error(err);
                                break Err(());
                            }
                        }
                    };

                    if res.is_err() {
                        debug!("processing state handler returned an error");
                        Impl::ctrl_ref_mut(&main_ctrl_rc).switch_to_available();
                    } else {
                        debug!("processing state handled");
                    }

                    res
                }.boxed_local()
            }
        )));

        ctrl.open_action = Some(
            app.lookup_action("open")
                .unwrap()
                .downcast::<gio::SimpleAction>()
                .unwrap(),
        );
    }
}
