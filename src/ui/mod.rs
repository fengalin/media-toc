use std::{cell::RefCell, rc::Rc};

mod audio_controller;
use self::audio_controller::AudioController;
mod audio_dispatcher;
use self::audio_dispatcher::AudioDispatcher;

pub mod chapters_boundaries;
pub use self::chapters_boundaries::ChaptersBoundaries;

mod chapter_tree_manager;
use self::chapter_tree_manager::ChapterTreeManager;

mod export_controller;
use self::export_controller::{ExportController, ExportControllerImpl};
mod export_dispatcher;
use self::export_dispatcher::ExportDispatcher;

mod info_controller;
use self::info_controller::InfoController;
mod info_dispatcher;
use self::info_dispatcher::InfoDispatcher;

mod image;
use self::image::Image;

pub mod main_controller;
pub use self::main_controller::{ControllerState, MainController};
pub mod main_dispatcher;
pub use self::main_dispatcher::MainDispatcher;

mod output_base_controller;
use self::output_base_controller::{
    MediaProcessor, OutputBaseController, OutputControllerImpl, OutputMediaFileInfo,
    ProcessingState, ProcessingType,
};
mod output_base_dispatcher;
use self::output_base_dispatcher::{OutputBaseDispatcher, OutputDispatcherImpl};

mod perspective_controller;
use self::perspective_controller::PerspectiveController;
mod perspective_dispatcher;
use self::perspective_dispatcher::PerspectiveDispatcher;

mod streams_controller;
use self::streams_controller::StreamsController;
mod streams_dispatcher;
use self::streams_dispatcher::StreamsDispatcher;

mod split_controller;
use self::split_controller::{SplitController, SplitControllerImpl};
mod split_dispatcher;
use self::split_dispatcher::SplitDispatcher;

mod ui_event;
use self::ui_event::{UIEvent, UIEventSender};

mod video_controller;
use self::video_controller::VideoController;
mod video_dispatcher;
use self::video_dispatcher::VideoDispatcher;

pub mod waveform_buffer;
pub use self::waveform_buffer::{DoubleWaveformBuffer, ImagePositions, WaveformBuffer};

pub mod waveform_image;
pub use self::waveform_image::{WaveformImage, BACKGROUND_COLOR};

#[derive(PartialEq)]
pub enum PositionStatus {
    ChapterChanged,
    ChapterNotChanged,
}

pub trait UIController {
    fn setup(&mut self) {}
    fn new_media(&mut self, _pipeline: &super::media::PlaybackPipeline) {}
    fn cleanup(&mut self);
    fn streams_changed(&mut self, _info: &super::metadata::MediaInfo) {}
}

pub trait UIDispatcher {
    fn setup(gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>);
}

/// This macro allows declaring a closure which will borrow the specified `main_ctrl_rc`
/// The following variantes are available:
///
/// - Borrow as immutable
/// ```
/// with_main_ctrl!(
///     main_ctrl_rc => |&main_ctrl| main_ctrl.about()
/// )
/// ```
///
/// - Borrow as mutable
/// ```
/// with_main_ctrl!(
///     main_ctrl_rc => |&mut main_ctrl| main_ctrl.quit()
/// )
/// ```
///
/// - Borrow as mutable with argument(s) (also available as immutable)
/// ```
/// with_main_ctrl!(
///     main_ctrl_rc => |&mut main_ctrl, event| main_ctrl.handle_media_event(event)
/// )
/// ```
///
/// - Try to borrow as mutable (also available with argument(s)). The body will not be called if
/// the borrow attempt fails.
/// ```
/// with_main_ctrl!(
///     main_ctrl_rc => try |&mut main_ctrl| main_ctrl.about()
/// )
/// ```
///
/// - Borrow as mutable and trigger asynchronously (also available as immutable and with argument(s))
/// ```
/// with_main_ctrl!(
///     main_ctrl_rc => async |&mut main_ctrl| main_ctrl.about()
/// )
/// ```
#[macro_export]
macro_rules! with_main_ctrl {
    (@param _) => ( _ );
    (@param $x:ident) => ( $x );
    ($main_ctrl_rc:ident => move |&$main_ctrl:ident| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move || {
                let $main_ctrl = main_ctrl_rc.borrow();
                $body
            }
        }
    );
    ($main_ctrl_rc:ident => move |&$main_ctrl:ident, $($p:tt),+| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move |$(with_main_ctrl!(@param $p),)+| {
                let $main_ctrl = main_ctrl_rc.borrow();
                $body
            }
        }
    );
    ($main_ctrl_rc:ident => try move |&$main_ctrl:ident| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move || {
                if let Ok($main_ctrl) = main_ctrl_rc.try_borrow() {
                    $body
                }
            }
        }
    );
    ($main_ctrl_rc:ident => try move |&$main_ctrl:ident, $($p:tt),+| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move |$(with_main_ctrl!(@param $p),)+| {
                if let Ok($main_ctrl) = main_ctrl_rc.try_borrow() {
                    $body
                }
            }
        }
    );
    ($main_ctrl_rc:ident => async move |&$main_ctrl:ident| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move || {
                let main_ctrl_rc_idle = Rc::clone(&main_ctrl_rc);
                gtk::idle_add(move || {
                    let $main_ctrl = main_ctrl_rc_idle.borrow();
                    $body
                })
            }
        }
    );
    ($main_ctrl_rc:ident => async move |&$main_ctrl:ident, $($p:tt),+| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move |$(with_main_ctrl!(@param $p),)+| {
                let main_ctrl_rc_idle = Rc::clone(&main_ctrl_rc);
                gtk::idle_add(move |$(with_main_ctrl!(@param $p),)+| {
                    let $main_ctrl = main_ctrl_rc_idle.borrow();
                    $body
                })
            }
        }
    );
    ($main_ctrl_rc:ident => move |&mut $main_ctrl:ident| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move || {
                let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                $body
            }
        }
    );
    ($main_ctrl_rc:ident => move |&mut $main_ctrl:ident, $($p:tt),+| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move |$(with_main_ctrl!(@param $p),)+| {
                let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                $body
            }
        }
    );
    ($main_ctrl_rc:ident => try move |&mut $main_ctrl:ident| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move || {
                if let Ok(mut $main_ctrl) = main_ctrl_rc.try_borrow_mut() {
                    $body
                }
            }
        }
    );
    ($main_ctrl_rc:ident => try move |&mut $main_ctrl:ident, $($p:tt),+| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move |$(with_main_ctrl!(@param $p),)+| {
                if let Ok(mut $main_ctrl) = main_ctrl_rc.try_borrow_mut() {
                    $body
                }
            }
        }
    );
    ($main_ctrl_rc:ident => async move |&mut $main_ctrl:ident| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move || {
                let main_ctrl_rc_idle = Rc::clone(&main_ctrl_rc);
                gtk::idle_add(move || {
                    let mut $main_ctrl = main_ctrl_rc_idle.borrow_mut();
                    $body
                })
            }
        }
    );
    ($main_ctrl_rc:ident => async move |&mut $main_ctrl:ident, $($p:tt),+| $body:expr) => (
        {
            let main_ctrl_rc = Rc::clone(&$main_ctrl_rc);
            move |$(with_main_ctrl!(@param $p),)+| {
                let main_ctrl_rc_idle = Rc::clone(&main_ctrl_rc);
                gtk::idle_add(move |$(with_main_ctrl!(@param $p),)+| {
                    let mut $main_ctrl = main_ctrl_rc_idle.borrow_mut();
                    $body
                })
            }
        }
    );
}
