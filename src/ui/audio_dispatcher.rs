use gdk;
use gdk::CursorType;
use gio;
use gio::prelude::*;
use gstreamer as gst;
use gtk;
use gtk::prelude::*;

use std::{cell::RefCell, rc::Rc};

use super::{
    AudioControllerAction, AudioControllerState, MainController, PositionStatus, UIDispatcher,
};

pub struct AudioDispatcher;
impl UIDispatcher for AudioDispatcher {
    fn setup(gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>) {
        let mut main_ctrl = main_ctrl_rc.borrow_mut();
        let audio_ctrl = &mut main_ctrl.audio_ctrl;

        // draw
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        audio_ctrl
            .drawingarea
            .connect_draw(move |drawing_area, cairo_ctx| {
                let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
                if let Some(position) = main_ctrl.audio_ctrl.draw(drawing_area, cairo_ctx) {
                    main_ctrl.refresh_info(position);
                }

                Inhibit(false)
            });

        // widget size changed
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        audio_ctrl
            .drawingarea
            .connect_size_allocate(move |_, alloc| {
                if let Ok(mut main_ctrl) = main_ctrl_rc_cb.try_borrow_mut() {
                    let mut audio_ctrl = &mut main_ctrl.audio_ctrl;
                    audio_ctrl.area_height = f64::from(alloc.height);
                    audio_ctrl.area_width = f64::from(alloc.width);
                    audio_ctrl.update_conditions();
                }
            });

        // Move cursor over drawing_area
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        audio_ctrl
            .drawingarea
            .connect_motion_notify_event(move |_, event_motion| {
                let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
                if let Some((boundary, position)) =
                    main_ctrl.audio_ctrl.motion_notify(&event_motion)
                {
                    if main_ctrl.move_chapter_boundary(boundary, position)
                        == PositionStatus::ChapterChanged
                    {
                        main_ctrl.audio_ctrl.state = AudioControllerState::MovingBoundary(position);
                        main_ctrl.audio_ctrl.redraw();
                    }
                }
                Inhibit(true)
            });

        // Leave drawing_area
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        audio_ctrl
            .drawingarea
            .connect_leave_notify_event(move |_, _event_crossing| {
                let main_ctrl = main_ctrl_rc_cb.borrow();
                let audio_ctrl = &main_ctrl.audio_ctrl;
                if let AudioControllerState::Paused = audio_ctrl.state {
                    audio_ctrl.reset_cursor();
                }
                Inhibit(true)
            });

        // button press in drawing_area
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        audio_ctrl
            .drawingarea
            .connect_button_press_event(move |_, event_button| {
                let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
                match main_ctrl.audio_ctrl.button_press(event_button) {
                    Some(AudioControllerAction::Seek(position)) => {
                        main_ctrl.seek(position, gst::SeekFlags::ACCURATE)
                    }
                    Some(AudioControllerAction::PlayRange {
                        start,
                        end,
                        current,
                    }) => {
                        main_ctrl.play_range(start, end, current);
                    }
                    _ => (),
                }
                Inhibit(true)
            });

        // button release in drawing_area
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        audio_ctrl
            .drawingarea
            .connect_button_release_event(move |_, event_button| {
                if 1 == event_button.get_button() {
                    // left button
                    let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
                    let mut audio_ctrl = &mut main_ctrl.audio_ctrl;
                    if let AudioControllerState::MovingBoundary(_boundary) = audio_ctrl.state {
                        audio_ctrl.state = AudioControllerState::Paused;

                        match audio_ctrl.get_boundary_at(event_button.get_position().0) {
                            Some(_boundary) => {
                                audio_ctrl.set_cursor(CursorType::SbHDoubleArrow);
                            }
                            None => {
                                audio_ctrl.reset_cursor();
                            }
                        }
                    }
                }
                Inhibit(true)
            });

        // Register Zoom in action
        let zoom_in = gio::SimpleAction::new("zoom_in", None);
        gtk_app.add_action(&zoom_in);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        zoom_in.connect_activate(move |_, _| {
            main_ctrl_rc_cb.borrow_mut().audio_ctrl.zoom_in();
        });
        gtk_app.set_accels_for_action("app.zoom_in", &["z"]);

        // Register Zoom out action
        let zoom_out = gio::SimpleAction::new("zoom_out", None);
        gtk_app.add_action(&zoom_out);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        zoom_out.connect_activate(move |_, _| {
            main_ctrl_rc_cb.borrow_mut().audio_ctrl.zoom_out();
        });
        gtk_app.set_accels_for_action("app.zoom_out", &["<Shift>z"]);

        // Register Step forward action
        let step_forward = gio::SimpleAction::new("step_forward", None);
        gtk_app.add_action(&step_forward);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        step_forward.connect_activate(move |_, _| {
            let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
            let seek_pos = main_ctrl.get_position() + main_ctrl.audio_ctrl.seek_step;
            main_ctrl.seek(seek_pos, gst::SeekFlags::ACCURATE);
        });
        gtk_app.set_accels_for_action("app.step_forward", &["Right"]);

        // Register Step back action
        let step_back = gio::SimpleAction::new("step_back", None);
        gtk_app.add_action(&step_back);
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        step_back.connect_activate(move |_, _| {
            let mut main_ctrl = main_ctrl_rc_cb.borrow_mut();
            let seek_pos = {
                let position = main_ctrl.get_position();
                let audio_ctrl = &mut main_ctrl.audio_ctrl;
                if audio_ctrl.current_position > audio_ctrl.seek_step {
                    position - audio_ctrl.seek_step
                } else {
                    0
                }
            };
            main_ctrl.seek(seek_pos, gst::SeekFlags::ACCURATE);
        });
        gtk_app.set_accels_for_action("app.step_back", &["Left"]);

        // Update conditions asynchronously
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        audio_ctrl.update_conditions_async = Some(Rc::new(move || {
            main_ctrl_rc_cb.borrow_mut().audio_ctrl.update_conditions();
        }));

        // Tick callback
        let main_ctrl_rc_cb = Rc::clone(main_ctrl_rc);
        audio_ctrl.tick_cb = Some(Rc::new(move |_da, _frame_clock| {
            main_ctrl_rc_cb.borrow_mut().audio_ctrl.tick();
        }));
    }
}
