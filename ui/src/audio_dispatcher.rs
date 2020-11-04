use gio::prelude::*;
use glib::clone;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

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
        ui_event: &UIEventSender,
    ) {
        // draw
        let waveform_with_overlay = RefCell::new(audio_ctrl.waveform_with_overlay());
        audio_ctrl
            .drawingarea
            .connect_draw(move |drawing_area, cairo_ctx| {
                waveform_with_overlay
                    .borrow_mut()
                    .draw(drawing_area, cairo_ctx);
                Inhibit(false)
            });

        // widget size changed
        audio_ctrl
            .drawingarea
            .connect_size_allocate(clone!(@strong ui_event => move |_, alloc| {
                ui_event.update_audio_rendering_cndt(Some((
                    f64::from(alloc.width),
                    f64::from(alloc.height),
                )));
            }));

        // Move cursor over drawing_area
        audio_ctrl.drawingarea.connect_motion_notify_event(
            clone!(@weak main_ctrl_rc => @default-panic, move |_, event_motion| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                if let Some((boundary, position)) =
                    main_ctrl.audio_ctrl.motion_notify(&event_motion)
                {
                    if let PositionStatus::ChapterChanged { .. } =
                        main_ctrl.move_chapter_boundary(boundary, position)
                    {
                        main_ctrl.audio_ctrl.state = ControllerState::MovingBoundary(position);
                        main_ctrl.audio_ctrl.drawingarea.queue_draw();
                    }
                }
                Inhibit(true)
            }),
        );

        // Leave drawing_area
        audio_ctrl.drawingarea.connect_leave_notify_event(
            clone!(@weak main_ctrl_rc => @default-panic, move |_, _event_crossing| {
                let main_ctrl = main_ctrl_rc.try_borrow_mut();
                match main_ctrl {
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
            clone!(@weak main_ctrl_rc => @default-panic, move |_, event_button| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.audio_ctrl.button_pressed(event_button);
                Inhibit(true)
            }),
        );

        // button release in drawing_area
        audio_ctrl.drawingarea.connect_button_release_event(
            clone!(@weak main_ctrl_rc => @default-panic, move |_, event_button| {
                let mut main_ctrl = main_ctrl_rc.borrow_mut();
                main_ctrl.audio_ctrl.button_released(event_button);
                Inhibit(true)
            }),
        );

        // Register Zoom in action
        app.add_action(&audio_ctrl.zoom_in_action);
        audio_ctrl
            .zoom_in_action
            .connect_activate(clone!(@strong ui_event => move |_, _| {
                ui_event.zoom_in()
            }));

        // Register Zoom out action
        app.add_action(&audio_ctrl.zoom_out_action);
        audio_ctrl
            .zoom_out_action
            .connect_activate(clone!(@strong ui_event => move |_, _| {
                ui_event.zoom_out()
            }));

        // Register Step forward action
        app.add_action(&audio_ctrl.step_forward_action);
        audio_ctrl
            .step_forward_action
            .connect_activate(clone!(@strong ui_event => move |_, _| {
                ui_event.step_forward();
            }));

        // Register Step back action
        app.add_action(&audio_ctrl.step_back_action);
        audio_ctrl
            .step_back_action
            .connect_activate(clone!(@strong ui_event => move |_, _| {
                ui_event.step_back();
            }));
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
