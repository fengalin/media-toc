use futures::{
    future::{self, LocalBoxFuture},
    prelude::*,
};

use gio::prelude::*;
use gtk::prelude::*;

use log::{debug, trace};

use std::cell::RefCell;

use crate::{audio, info::PositionStatus, main, prelude::*};
use renderers::Timestamp;

use super::AreaEvent;

pub struct Dispatcher;

impl UIDispatcher for Dispatcher {
    type Controller = audio::Controller;
    type Event = audio::Event;

    fn setup(audio: &mut audio::Controller, app: &gtk::Application) {
        use audio::AreaEvent::*;

        // draw
        let waveform_with_overlay = RefCell::new(audio.waveform_with_overlay());
        audio
            .drawingarea
            .connect_draw(move |drawing_area, cairo_ctx| {
                waveform_with_overlay
                    .borrow_mut()
                    .draw(drawing_area, cairo_ctx);
                Inhibit(false)
            });

        // widget size changed
        audio.drawingarea.connect_size_allocate(|_, alloc| {
            audio::update_rendering_cndt(Some((f64::from(alloc.width), f64::from(alloc.height))));
        });

        // Move cursor over drawing_area
        audio.drawingarea.connect_motion_notify_event(|_, event| {
            audio::area_event(Motion(event.clone()));
            Inhibit(true)
        });

        // Leave drawing_area
        audio.drawingarea.connect_leave_notify_event(|_, _event| {
            audio::area_event(Leaving);
            Inhibit(true)
        });

        // button press in drawing_area
        audio.drawingarea.connect_button_press_event(|_, event| {
            audio::area_event(Button(event.clone()));
            Inhibit(true)
        });

        // button release in drawing_area
        audio.drawingarea.connect_button_release_event(|_, event| {
            audio::area_event(Button(event.clone()));
            Inhibit(true)
        });

        // Register Zoom in action
        app.add_action(&audio.zoom_in_action);
        audio
            .zoom_in_action
            .connect_activate(|_, _| audio::zoom_in());

        // Register Zoom out action
        app.add_action(&audio.zoom_out_action);
        audio
            .zoom_out_action
            .connect_activate(|_, _| audio::zoom_out());

        // Register Step forward action
        app.add_action(&audio.step_forward_action);
        audio
            .step_forward_action
            .connect_activate(|_, _| audio::step_forward());

        // Register Step back action
        app.add_action(&audio.step_back_action);
        audio
            .step_back_action
            .connect_activate(|_, _| audio::step_back());
    }

    fn handle_event(
        main_ctrl: &mut main::Controller,
        event: impl Into<Self::Event>,
    ) -> LocalBoxFuture<'_, ()> {
        use audio::Event::*;

        let event = event.into();
        if let Tick | Area(_) = event {
            trace!("handling {:?}", event);
        } else {
            debug!("handling {:?}", event);
        }
        match event {
            Area(event) => Self::area_event(main_ctrl, event),
            UpdateRenderingCndt(dimensions) => main_ctrl.audio.update_conditions(dimensions),
            Refresh => main_ctrl.audio.refresh(),
            StepBack => Self::step_back(main_ctrl),
            StepForward => Self::step_forward(main_ctrl),
            Tick => main_ctrl.audio.tick(),
            ZoomIn => main_ctrl.audio.zoom_in(),
            ZoomOut => main_ctrl.audio.zoom_out(),
        }

        future::ready(()).boxed_local()
    }

    fn bind_accels_for(ctx: UIFocusContext, app: &gtk::Application) {
        use UIFocusContext::*;

        match ctx {
            PlaybackPage => {
                app.set_accels_for_action("app.zoom_in", &["z"]);
                app.set_accels_for_action("app.zoom_out", &["<Shift>z"]);
                app.set_accels_for_action("app.step_forward", &["Right"]);
                app.set_accels_for_action("app.step_back", &["Left"]);
            }
            ExportPage | InfoBar | StreamsPage | SplitPage | TextEntry => {
                app.set_accels_for_action("app.zoom_in", &[]);
                app.set_accels_for_action("app.zoom_out", &[]);
                app.set_accels_for_action("app.step_forward", &[]);
                app.set_accels_for_action("app.step_back", &[]);
            }
        }
    }
}

impl Dispatcher {
    pub fn area_event(main_ctrl: &mut main::Controller, event: AreaEvent) {
        use AreaEvent::*;

        match event {
            Button(event) => match event.get_event_type() {
                gdk::EventType::ButtonPress => main_ctrl.audio.button_pressed(event),
                gdk::EventType::ButtonRelease => main_ctrl.audio.button_released(event),
                gdk::EventType::Scroll => {
                    // FIXME zoom in / out
                }
                _ => (),
            },
            Leaving => main_ctrl.audio.leave_drawing_area(),
            Motion(event) => {
                if let Some((boundary, target)) = main_ctrl.audio.motion_notify(event) {
                    if let PositionStatus::ChapterChanged { .. } =
                        main_ctrl.info.move_chapter_boundary(boundary, target)
                    {
                        // FIXME this is ugly
                        main_ctrl.audio.state = audio::controller::State::MovingBoundary(target);
                        main_ctrl.audio.drawingarea.queue_draw();
                    }
                }
            }
        }
    }

    pub fn step_back(main_ctrl: &mut main::Controller) {
        if let Some(current_ts) = main_ctrl.current_ts() {
            let seek_ts = {
                let seek_step = main_ctrl.audio.seek_step;
                if current_ts > seek_step {
                    current_ts - seek_step
                } else {
                    Timestamp::default()
                }
            };
            let _ = main_ctrl.seek(seek_ts, gst::SeekFlags::ACCURATE);
        }
    }

    pub fn step_forward(main_ctrl: &mut main::Controller) {
        if let Some(current_ts) = main_ctrl.current_ts() {
            let seek_ts = current_ts + main_ctrl.audio.seek_step;
            let _ = main_ctrl.seek(seek_ts, gst::SeekFlags::ACCURATE);
        }
    }
}
