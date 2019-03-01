use gstreamer as gst;

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
pub use self::timestamp::{parse_timestamp, Timestamp};

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaContent {
    Audio,
    AudioVideo,
    AudioText,
    AudioVideoText,
    Text,
    Video,
    VideoText,
    Undefined,
}

impl MediaContent {
    pub fn add_stream_type(&mut self, type_: gst::StreamType) {
        match type_ {
            gst::StreamType::AUDIO => match self {
                MediaContent::Text => *self = MediaContent::AudioText,
                MediaContent::Video => *self = MediaContent::AudioVideo,
                MediaContent::VideoText => *self = MediaContent::AudioVideoText,
                MediaContent::Undefined => *self = MediaContent::Audio,
                _ => (),
            },
            gst::StreamType::VIDEO => match self {
                MediaContent::Audio => *self = MediaContent::AudioVideo,
                MediaContent::Text => *self = MediaContent::VideoText,
                MediaContent::AudioText => *self = MediaContent::AudioVideoText,
                MediaContent::Undefined => *self = MediaContent::Video,
                _ => (),
            },
            gst::StreamType::TEXT => match self {
                MediaContent::Audio => *self = MediaContent::AudioText,
                MediaContent::Video => *self = MediaContent::VideoText,
                MediaContent::AudioVideo => *self = MediaContent::AudioVideoText,
                MediaContent::Undefined => *self = MediaContent::Text,
                _ => (),
            },
            _ => panic!("MediaContent::add_stream_type can't handle {:?}", type_),
        };
    }
}

impl Default for MediaContent {
    fn default() -> Self {
        MediaContent::Undefined
    }
}
