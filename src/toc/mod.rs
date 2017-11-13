pub mod chapter;
pub use self::chapter::Chapter;

pub mod cue_sheet_format;
pub use self::cue_sheet_format::CueSheetFormat;

pub mod exporter;
pub use self::exporter::Exporter;

pub mod factory;
pub use self::factory::Factory;

pub mod importer;
pub use self::importer::Importer;

pub mod matroska_toc_format;
pub use self::matroska_toc_format::MatroskaTocFormat;

pub mod mkvmerge_text_format;
pub use self::mkvmerge_text_format::MKVMergeTextFormat;

pub mod timestamp;
pub use self::timestamp::Timestamp;

pub static METADATA_ARTIST: &'static str = "artist";
pub static METADATA_AUDIO_CODEC: &'static str = "audio_codec";
pub static METADATA_CONTAINER: &'static str = "media_container";
pub static METADATA_FILE_NAME: &'static str = "file_name";
pub static METADATA_TITLE: &'static str = "title";
pub static METADATA_VIDEO_CODEC: &'static str = "video_codec";

#[derive(Clone, PartialEq)]
pub enum Format {
    CueSheet,
    Matroska,
    MKVMergeText,
}
