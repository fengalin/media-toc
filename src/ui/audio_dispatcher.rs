use futures::prelude::*;

use gio;
use gio::prelude::*;
use glib::clone;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use media::Timestamp;

use super::{
    audio_controller::ControllerState, AudioController, MainController, PositionStatus,
    UIDispatcher, UIEventSender, UIFocusContext,
};

pub struct AudioDispatcher;
impl UIDispatcher for AudioDispatcher {
    type Controller = AudioController;

    fn setup(
        audio_ctrl: &mut AudioController,
        main_ctrl_rc: &Rc<RefCell<MainController>>,
        app: &gtk::Application,
        _ui_event: &UIEventSender,
    ) {
        // draw
        audio_ctrl.drawingarea.connect_draw(clone!(
            @strong main_ctrl_rc => move |drawing_area, cairo_ctx| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                if let Some(position) = main_ctrl.audio_ctrl.draw(drawing_area, cairo_ctx) {
                    main_ctrl.refresh_info(position);
                }
                Inhibit(false)
            }
        ));

        // widget size changed
        audio_ctrl.drawingarea.connect_size_allocate(
            clone!(@strong main_ctrl_rc => move |_, alloc| {
                if let Ok(mut main_ctrl) = main_ctrl_rc.try_borrow_mut() {
                    let mut audio_ctrl = &mut main_ctrl.audio_ctrl;
                    audio_ctrl.area_height = f64::from(alloc.height);
                    audio_ctrl.area_width = f64::from(alloc.width);
                    audio_ctrl.update_conditions();
                }
            }),
        );

        // Move cursor over drawing_area
        audio_ctrl.drawingarea.connect_motion_notify_event(
            clone!(@strong main_ctrl_rc => move |_, event_motion| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                if let Some((boundary, position)) =
                    main_ctrl.audio_ctrl.motion_notify(&event_motion)
                {
                    if let PositionStatus::ChapterChanged { .. } =
                        main_ctrl.move_chapter_boundary(boundary, position)
                    {
                        main_ctrl.audio_ctrl.state = ControllerState::MovingBoundary(position);
                        main_ctrl.audio_ctrl.redraw();
                    }
                }
                Inhibit(true)
            }),
        );

        // Leave drawing_area
        audio_ctrl.drawingarea.connect_leave_notify_event(
            clone!(@strong main_ctrl_rc => move |_, _event_crossing| {
                match main_ctrl_rc.try_borrow_mut() {
                    Ok(mut main_ctrl) => {
                        main_ctrl.audio_ctrl.leave_drawing_area();
                        Inhibit(true)
                    }
                    Err(_) => Inhibit(false),
                }
            }),
        );

        // button press in drawing_area
        audio_ctrl.drawingarea.connect_button_press_event(
            clone!(@strong main_ctrl_rc => move |_, event_button| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.audio_ctrl.button_pressed(event_button);
                Inhibit(true)
            }),
        );

        // button release in drawing_area
        audio_ctrl.drawingarea.connect_button_release_event(
            clone!(@strong main_ctrl_rc => move |_, event_button| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.audio_ctrl.button_released(event_button);
                Inhibit(true)
            }),
        );

        // Register Zoom in action
        app.add_action(&audio_ctrl.zoom_in_action);
        audio_ctrl
            .zoom_in_action
            .connect_activate(clone!(@strong main_ctrl_rc => move |_, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.audio_ctrl.zoom_in()
            }));

        // Register Zoom out action
        app.add_action(&audio_ctrl.zoom_out_action);
        audio_ctrl
            .zoom_out_action
            .connect_activate(clone!(@strong main_ctrl_rc => move |_, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.audio_ctrl.zoom_out()
            }));

        // Register Step forward action
        app.add_action(&audio_ctrl.step_forward_action);
        audio_ctrl.step_forward_action.connect_activate(
            clone!(@strong main_ctrl_rc => move |_, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                let seek_target = main_ctrl.get_current_ts() + main_ctrl.audio_ctrl.seek_step;
                main_ctrl.seek(seek_target, gst::SeekFlags::ACCURATE);
            }),
        );

        // Register Step back action
        app.add_action(&audio_ctrl.step_back_action);
        audio_ctrl
            .step_back_action
            .connect_activate(clone!(@strong main_ctrl_rc => move |_, _| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                let seek_pos = {
                    let ts = main_ctrl.get_current_ts();
                    let audio_ctrl = &mut main_ctrl.audio_ctrl;
                    if ts > audio_ctrl.seek_step {
                        ts - audio_ctrl.seek_step
                    } else {
                        Timestamp::default()
                    }
                };
                main_ctrl.seek(seek_pos, gst::SeekFlags::ACCURATE);
            }));

        // Update conditions asynchronously
        audio_ctrl.update_conditions_async =
            Some(Box::new(clone!(@strong main_ctrl_rc => move || {
                let main_ctrl_rc = Rc::clone(&main_ctrl_rc);
                async move {
                    let mut main_ctrl = main_ctrl_rc.borrow_mut();
                    main_ctrl.audio_ctrl.update_conditions();
                }.boxed_local()
            })));

        // Tick callback
        audio_ctrl.tick_cb = Some(Rc::new(
            clone!(@strong main_ctrl_rc => move |_da, _frame_clock| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.audio_ctrl.tick();
            }),
        ));
    }

    fn bind_accels_for(ctx: UIFocusContext, app: &gtk::Application) {
        match ctx {
            UIFocusContext::PlaybackPage => {
                app.set_accels_for_action("app.zoom_in", &["z"]);
                app.set_accels_for_action("app.zoom_out", &["<Shift>z"]);
                app.set_accels_for_action("app.step_forward", &["Right"]);
                app.set_accels_for_action("app.step_back", &["Left"]);
            }
            UIFocusContext::ExportPage
            | UIFocusContext::InfoBar
            | UIFocusContext::StreamsPage
            | UIFocusContext::SplitPage
            | UIFocusContext::TextEntry => {
                app.set_accels_for_action("app.zoom_in", &[]);
                app.set_accels_for_action("app.zoom_out", &[]);
                app.set_accels_for_action("app.step_forward", &[]);
                app.set_accels_for_action("app.step_back", &[]);
            }
        }
    }
}
