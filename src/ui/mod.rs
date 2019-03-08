use std::{cell::RefCell, rc::Rc};

mod audio_controller;
use self::audio_controller::AudioController;

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

mod image;
use self::image::Image;

pub mod main_controller;
pub use self::main_controller::{ControllerState, MainController};

mod output_base_controller;
use self::output_base_controller::{
    MediaProcessor, OutputBaseController, OutputControllerImpl, OutputMediaFileInfo,
    ProcessingStatus, ProcessingType,
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
    // FIXME: remove Rc<RefCell<Self>> version when all controllers are no longer Rcs
    fn setup_(
        _this_rc: &Rc<RefCell<Self>>,
        _gtk_app: &gtk::Application,
        _main_ctrl_rc: &Rc<RefCell<MainController>>,
    ) {
    }
    fn setup(&mut self) {}
    fn new_media(&mut self, pipeline: &super::media::PlaybackPipeline);
    fn cleanup(&mut self);
    fn streams_changed(&mut self, _info: &super::metadata::MediaInfo) {}
}

pub trait UIDispatcher {
    fn setup(gtk_app: &gtk::Application, main_ctrl_rc: &Rc<RefCell<MainController>>);
}
