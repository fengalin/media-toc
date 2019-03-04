use std::{cell::RefCell, rc::Rc};

mod audio_controller;
use self::audio_controller::AudioController;

pub mod chapters_boundaries;
pub use self::chapters_boundaries::ChaptersBoundaries;

mod chapter_tree_manager;
use self::chapter_tree_manager::ChapterTreeManager;

mod export_controller;
use self::export_controller::ExportController;

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

mod perspective_controller;
use self::perspective_controller::PerspectiveController;

mod streams_controller;
use self::streams_controller::StreamsController;

mod split_controller;
use self::split_controller::SplitController;

mod video_controller;
use self::video_controller::VideoController;

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
    fn setup(
        this_rc: &Rc<RefCell<Self>>,
        gtk_app: &gtk::Application,
        main_ctrl: &Rc<RefCell<MainController>>,
    );
    fn new_media(&mut self, pipeline: &super::media::PlaybackPipeline);
    fn cleanup(&mut self);
    fn streams_changed(&mut self, _info: &super::metadata::MediaInfo) {}
}
