mod audio_controller;
use self::audio_controller::AudioController;

mod info_controller;
use self::info_controller::InfoController;

mod image_surface;
use self::image_surface::ImageSurface;

pub mod main_controller;
pub use self::main_controller::{ControllerState, MainController};

mod video_controller;
use self::video_controller::VideoController;

pub mod waveform_buffer;
pub use self::waveform_buffer::{DoubleWaveformBuffer, WaveformConditions, WaveformBuffer};

pub mod waveform_image;
pub use self::waveform_image::{BACKGROUND_COLOR, WaveformImage};
