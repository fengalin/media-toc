pub const APP_ID: &str = "org.fengalin.media-toc";

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

mod image_surface;
use self::image_surface::ImageSurface;

pub mod main_controller;
pub use self::main_controller::{ControllerState, MainController};

mod output_base_controller;
use self::output_base_controller::OutputBaseController;

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
