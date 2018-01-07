pub mod chapter;
pub use self::chapter::Chapter;

pub mod cue_sheet_format;
pub use self::cue_sheet_format::CueSheetFormat;

pub mod factory;
pub use self::factory::Factory;

pub mod format;
pub use self::format::{Exporter, Reader, Writer};

pub mod matroska_toc_format;
pub use self::matroska_toc_format::MatroskaTocFormat;

pub mod media_info;
pub use self::media_info::MediaInfo;

pub mod mkvmerge_text_format;
pub use self::mkvmerge_text_format::MKVMergeTextFormat;

pub mod timestamp;
pub use self::timestamp::Timestamp;

pub static METADATA_TITLE: &'static str = "title";

pub static DEFAULT_TITLE: &'static str = "untitled";

#[derive(Clone, Debug, PartialEq)]
pub enum Format {
    CueSheet,
    Flac,
    Matroska,
    MKVMergeText,
    Wave,
}
