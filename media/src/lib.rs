pub mod pipeline;
pub use pipeline::{MissingPlugins, OpenError, SeekError, SelectStreamsError};

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
