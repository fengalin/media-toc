pub mod audio_channel;
pub use self::audio_channel::{AudioChannel, AudioChannelSide, INLINE_CHANNELS};

pub mod audio_buffer;
pub use self::audio_buffer::AudioBuffer;

pub mod dbl_audio_buffer;
pub use self::dbl_audio_buffer::DoubleAudioBuffer;

pub mod playback_pipeline;
pub use self::playback_pipeline::{
    MissingPlugins, OpenError, PlaybackPipeline, SeekError, SelectStreamsError, StateChangeError,
    QUEUE_SIZE,
};

pub mod sample_extractor;
pub use self::sample_extractor::SampleExtractor;

pub mod sample_index;
pub use self::sample_index::SampleIndex;

pub mod sample_index_range;
pub use self::sample_index_range::SampleIndexRange;

pub mod sample_value;
pub use self::sample_value::SampleValue;

pub mod splitter_pipeline;
pub use self::splitter_pipeline::SplitterPipeline;

pub mod timestamp;
pub use self::timestamp::Timestamp;

pub mod toc_setter_pipeline;
pub use self::toc_setter_pipeline::TocSetterPipeline;

#[derive(Clone, Debug)]
pub enum MediaEvent {
    Eos,
    Error(String),
    ReadyToRefresh,
    // FIXME remove those
    AsyncDone,
    FailedToExport(String),
    InitDone,
}
