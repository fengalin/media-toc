pub mod controller;
pub use controller::{AreaEvent, Controller};

mod dispatcher;
pub use dispatcher::Dispatcher;

mod waveform_with_overlay;
pub use waveform_with_overlay::WaveformWithOverlay;

use crate::UIEventChannel;

#[derive(Debug)]
pub enum Event {
    AreaEvent(AreaEvent),
    UpdateRenderingCndt(Option<(f64, f64)>),
    Refresh,
    // FIXME those 2 are not audio specific, rather for a dedicated playback
    StepBack,
    StepForward,
    Tick,
    ZoomIn,
    ZoomOut,
}

fn area_event(event: AreaEvent) {
    UIEventChannel::send(Event::AreaEvent(event));
}

pub fn update_rendering_cndt(dimensions: Option<(f64, f64)>) {
    UIEventChannel::send(Event::UpdateRenderingCndt(dimensions));
}

pub fn refresh() {
    UIEventChannel::send(Event::Refresh);
}

pub fn step_back() {
    UIEventChannel::send(Event::StepBack);
}

pub fn step_forward() {
    UIEventChannel::send(Event::StepForward);
}

pub fn tick() {
    UIEventChannel::send(Event::Tick);
}

fn zoom_in() {
    UIEventChannel::send(Event::ZoomIn);
}

fn zoom_out() {
    UIEventChannel::send(Event::ZoomOut);
}
