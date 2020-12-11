pub mod playback_pipeline;
pub use self::playback_pipeline::{
    MissingPlugins, OpenError, PlaybackPipeline, SeekError, SelectStreamsError, StateChangeError,
};

pub mod splitter_pipeline;
pub use self::splitter_pipeline::SplitterPipeline;

pub mod toc_setter_pipeline;
pub use self::toc_setter_pipeline::TocSetterPipeline;

use metadata::Duration;

/// Max duration that queues can hold.
pub const QUEUE_SIZE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub enum MediaEvent {
    AsyncDone,
    Eos,
    Error(String),
    MustRefresh,
    FailedToExport(String),
    InitDone,
    StateChanged,
}
