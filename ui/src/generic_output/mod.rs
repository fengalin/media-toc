mod controller;
pub use self::controller::{
    Controller, MediaEventHandling, MediaProcessorError, MediaProcessorImpl, OutputControllerImpl,
    OutputMediaFileInfo, ProcessingType, MEDIA_EVENT_CHANNEL_CAPACITY,
};

mod dispatcher;
pub use self::dispatcher::OutputDispatcher;

#[derive(Debug)]
pub enum Event {
    ActionOver,
    TriggerAction,
}

pub mod prelude {
    pub use super::controller::{
        MediaEventHandling, MediaProcessorError, MediaProcessorImpl, OutputControllerImpl,
        OutputMediaFileInfo, ProcessingType, MEDIA_EVENT_CHANNEL_CAPACITY,
    };
    pub use super::dispatcher::OutputDispatcher;
}
