use std::boxed::Box;

use super::{
    CueSheetFormat, Format, MKVMergeTextFormat, MatroskaTocFormat, MediaContent, Reader, Writer,
};

pub struct Factory {}

impl Factory {
    pub fn extensions() -> Vec<(&'static str, Format)> {
        let mut result = Vec::<(&'static str, Format)>::new();

        // Only MKVMergeTextFormat implemented for Read ATM
        result.push((MKVMergeTextFormat::extension(), Format::MKVMergeText));

        result
    }

    pub fn extension(format: Format, content: MediaContent) -> &'static str {
        match format {
            Format::CueSheet => CueSheetFormat::extension(),
            Format::Flac => "flac",
            Format::Matroska => match content {
                MediaContent::Audio => MatroskaTocFormat::audio_extension(),
                _ => MatroskaTocFormat::extension(),
            },
            Format::MKVMergeText => MKVMergeTextFormat::extension(),
            Format::MP3 => "mp3",
            Format::Opus => "opus",
            Format::Vorbis => "oga",
            Format::Wave => "wave",
        }
    }

    pub fn reader(format: Format) -> Box<dyn Reader> {
        match format {
            Format::MKVMergeText => Box::new(MKVMergeTextFormat::default()),
            format => unimplemented!("Reader for {:?}", format),
        }
    }

    pub fn writer(format: Format) -> Box<dyn Writer> {
        match format {
            Format::CueSheet => Box::new(CueSheetFormat::default()),
            Format::MKVMergeText => Box::new(MKVMergeTextFormat::default()),
            format => unimplemented!("Writer for {:?}", format),
        }
    }
}
