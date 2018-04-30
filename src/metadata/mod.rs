pub mod cue_sheet_format;
pub use self::cue_sheet_format::CueSheetFormat;

pub mod factory;
pub use self::factory::Factory;

pub mod format;
pub use self::format::{Exporter, Reader, Writer};

pub mod matroska_toc_format;
pub use self::matroska_toc_format::MatroskaTocFormat;

pub mod media_info;
pub use self::media_info::{get_default_chapter_title, MediaInfo, Stream, Streams};

pub mod mkvmerge_text_format;
pub use self::mkvmerge_text_format::MKVMergeTextFormat;

pub mod timestamp;
pub use self::timestamp::{Timestamp, parse_timestamp};

pub mod toc_visitor;
pub use self::toc_visitor::{TocVisit, TocVisitor};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    CueSheet,
    Flac,
    Matroska,
    MKVMergeText,
    MP3,
    Opus,
    Vorbis,
    Wave,
}
