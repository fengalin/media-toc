use std::boxed::Box;

use super::{CueSheetFormat, Format, MatroskaTocFormat, MKVMergeTextFormat, Reader, Writer};

pub struct Factory {
}

impl Factory {
    pub fn get_extensions() -> Vec<(&'static str, Format)> {
        let mut result = Vec::<(&'static str, Format)>::new();

        result.push((MKVMergeTextFormat::get_extension(), Format::MKVMergeText));
        result.push((CueSheetFormat::get_extension(), Format::CueSheet));

        result
    }

    pub fn get_extension(format: &Format, is_audio_only: bool) -> &'static str {
        match *format {
            Format::CueSheet => CueSheetFormat::get_extension(),
            Format::Matroska =>
                if !is_audio_only {
                    MatroskaTocFormat::get_extension()
                } else {
                    MatroskaTocFormat::get_audio_extension()
                },
            Format::MKVMergeText => MKVMergeTextFormat::get_extension(),
        }
    }

    pub fn get_reader(format: &Format) -> Box<Reader> {
        match *format {
            Format::CueSheet => unimplemented!("Reader for toc::Format::CueSheet"),
            Format::Matroska => unimplemented!("Reader for toc::Format::Matroska"),
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
        }
    }

    pub fn get_writer(format: &Format) -> Box<Writer> {
        match *format {
            Format::CueSheet => CueSheetFormat::new_as_boxed(),
            Format::Matroska => unimplemented!("Reader for toc::Format::Matroska"),
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
        }
    }
}
