pub mod chapter;
pub use self::chapter::Chapter;

pub mod cue_sheet_format;
pub use self::cue_sheet_format::CueSheetFormat;

pub mod exporter;
pub use self::exporter::Exporter;

pub mod factory;
pub use self::factory::Factory;

pub mod mkvmerge_text_format;
pub use self::mkvmerge_text_format::MKVMergeTextFormat;

pub mod timestamp;
pub use self::timestamp::Timestamp;

pub const METADATA_ARTIST: &str = "artist";
pub const METADATA_AUDIO_CODEC: &str = "audio_codec";
pub const METADATA_CONTAINER: &str = "media_container";
pub const METADATA_FILE_NAME: &str = "file_name";
pub const METADATA_TITLE: &str = "title";
pub const METADATA_VIDEO_CODEC: &str = "video_codec";

#[derive(Clone, PartialEq)]
pub enum Format {
    MKVMergeText,
    CueSheet,
}
