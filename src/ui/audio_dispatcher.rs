use gio;
use gio::prelude::*;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use media::Timestamp;

use super::{audio_controller::ControllerState, MainController, PositionStatus, UIDispatcher};
use crate::with_main_ctrl;

pub struct AudioDispatcher;
impl UIDispatcher for AudioDispatcher {
    fn setup(gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>) {
        let mut main_ctrl = main_ctrl_rc.borrow_mut();
        let audio_ctrl = &mut main_ctrl.audio_ctrl;

        // draw
        audio_ctrl.drawingarea.connect_draw(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, drawing_area, cairo_ctx| {
                if let Some(position) = main_ctrl.audio_ctrl.draw(drawing_area, cairo_ctx) {
                    main_ctrl.refresh_info(position);
                }
                Inhibit(false)
            }
        ));

        // widget size changed
        audio_ctrl
            .drawingarea
            .connect_size_allocate(with_main_ctrl!(
                main_ctrl_rc => try move |&mut main_ctrl, _, alloc| {
                    let mut audio_ctrl = &mut main_ctrl.audio_ctrl;
                    audio_ctrl.area_height = f64::from(alloc.height);
                    audio_ctrl.area_width = f64::from(alloc.width);
                    audio_ctrl.update_conditions();
                }
            ));

        // Move cursor over drawing_area
        audio_ctrl
            .drawingarea
            .connect_motion_notify_event(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, event_motion| {
                    if let Some((boundary, position)) =
                        main_ctrl.audio_ctrl.motion_notify(&event_motion)
                    {
                        if let PositionStatus::ChapterChanged(_) =
                            main_ctrl.move_chapter_boundary(boundary, position)
                        {
                            main_ctrl.audio_ctrl.state = ControllerState::MovingBoundary(position);
                            main_ctrl.audio_ctrl.redraw();
                        }
                    }
                    Inhibit(true)
                }
            ));

        // Leave drawing_area
        audio_ctrl
            .drawingarea
            .connect_leave_notify_event(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, _event_crossing| {
                    main_ctrl.audio_ctrl.leave_drawing_area();
                    Inhibit(true)
                }
            ));

        // button press in drawing_area
        audio_ctrl
            .drawingarea
            .connect_button_press_event(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, event_button| {
                    main_ctrl.audio_ctrl.button_pressed(event_button);
                    Inhibit(true)
                }
            ));

        // button release in drawing_area
        audio_ctrl
            .drawingarea
            .connect_button_release_event(with_main_ctrl!(
                main_ctrl_rc => move |&mut main_ctrl, _, event_button| {
                    main_ctrl.audio_ctrl.button_released(event_button);
                    Inhibit(true)
                }
            ));

        // Register Zoom in action
        let zoom_in = gio::SimpleAction::new("zoom_in", None);
        gtk_app.add_action(&zoom_in);
        zoom_in.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| main_ctrl.audio_ctrl.zoom_in()
        ));
        gtk_app.set_accels_for_action("app.zoom_in", &["z"]);

        // Register Zoom out action
        let zoom_out = gio::SimpleAction::new("zoom_out", None);
        gtk_app.add_action(&zoom_out);
        zoom_out.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| main_ctrl.audio_ctrl.zoom_out()
        ));
        gtk_app.set_accels_for_action("app.zoom_out", &["<Shift>z"]);

        // Register Step forward action
        let step_forward = gio::SimpleAction::new("step_forward", None);
        gtk_app.add_action(&step_forward);
        step_forward.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| {
                let seek_target = main_ctrl.get_current_ts() + main_ctrl.audio_ctrl.seek_step;
                main_ctrl.seek(seek_target, gst::SeekFlags::ACCURATE);
            }
        ));
        gtk_app.set_accels_for_action("app.step_forward", &["Right"]);

        // Register Step back action
        let step_back = gio::SimpleAction::new("step_back", None);
        gtk_app.add_action(&step_back);
        step_back.connect_activate(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _, _| {
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
            }
        ));
        gtk_app.set_accels_for_action("app.step_back", &["Left"]);

        // Update conditions asynchronously
        audio_ctrl.update_conditions_async = Some(Box::new(with_main_ctrl!(
            main_ctrl_rc => async move |&mut main_ctrl| {
                main_ctrl.audio_ctrl.update_conditions();
                glib::Continue(false)
            }
        )));

        // Tick callback
        audio_ctrl.tick_cb = Some(Rc::new(with_main_ctrl!(
            main_ctrl_rc => move |&mut main_ctrl, _da, _frame_clock| {
                main_ctrl.audio_ctrl.tick();
            }
        )));
    }
}
