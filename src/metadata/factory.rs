use std::boxed::Box;

use super::{CueSheetFormat, Format, MKVMergeTextFormat, MatroskaTocFormat, Reader, Writer};

pub struct Factory {}

impl Factory {
    pub fn get_extensions() -> Vec<(&'static str, Format)> {
        let mut result = Vec::<(&'static str, Format)>::new();

        // Only MKVMergeTextFormat implemented for Read ATM
        result.push((MKVMergeTextFormat::get_extension(), Format::MKVMergeText));

        result
    }

    pub fn get_extension(format: &Format, is_audio_only: bool) -> &'static str {
        match *format {
            Format::CueSheet => CueSheetFormat::get_extension(),
            Format::Flac => "flac",
            Format::Matroska => {
                if !is_audio_only {
                    MatroskaTocFormat::get_extension()
                } else {
                    MatroskaTocFormat::get_audio_extension()
                }
            }
            Format::MKVMergeText => MKVMergeTextFormat::get_extension(),
            Format::MP3 => "mp3",
            Format::Opus => "opus",
            Format::Vorbis => "oga",
            Format::Wave => "wave",
        }
    }

    pub fn get_reader(format: &Format) -> Box<Reader> {
        match *format {
            Format::CueSheet => unimplemented!("Reader for toc::Format::CueSheet"),
            Format::Flac => unimplemented!("Reader for toc::Format::Flac"),
            Format::Matroska => unimplemented!("Reader for toc::Format::Matroska"),
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
            Format::MP3 => unimplemented!("Reader for toc::Format::MP3"),
            Format::Opus => unimplemented!("Reader for toc::Format::Opus"),
            Format::Vorbis => unimplemented!("Reader for toc::Format::Vorbis"),
            Format::Wave => unimplemented!("Reader for toc::Format::Wave"),
        }
    }

    pub fn get_writer(format: &Format) -> Box<Writer> {
        match *format {
            Format::CueSheet => CueSheetFormat::new_as_boxed(),
            Format::Flac => unimplemented!("Writer for toc::Format::Wave"),
            Format::Matroska => unimplemented!("Writer for toc::Format::Matroska"),
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
            Format::MP3 => unimplemented!("Writer for toc::Format::MP3"),
            Format::Opus => unimplemented!("Writer for toc::Format::Opus"),
            Format::Vorbis => unimplemented!("Writer for toc::Format::Vorbis"),
            Format::Wave => unimplemented!("Writer for toc::Format::Wave"),
        }
    }
}
