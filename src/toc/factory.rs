use std::boxed::Box;

use super::{CueSheetFormat, Exporter, Format, Importer, MatroskaTocFormat, MKVMergeTextFormat};

pub struct Factory {
}

impl Factory {
    pub fn get_extensions() -> Vec<(&'static str, Format)> {
        let mut result = Vec::<(&'static str, Format)>::new();

        result.push((MKVMergeTextFormat::get_extension(), Format::MKVMergeText));
        result.push((CueSheetFormat::get_extension(), Format::CueSheet));

        result
    }

    pub fn get_importer(format: Format) -> Box<Importer> {
        match format {
            Format::CueSheet => unimplemented!("Importer for toc::Format::CueSheet"),
            Format::Matroska => unimplemented!("Importer for toc::Format::Matroska"),
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
        }
    }

    pub fn get_exporter(format: Format) -> Box<Exporter> {
        match format {
            Format::CueSheet => CueSheetFormat::new_as_boxed(),
            Format::Matroska => MatroskaTocFormat::new_as_boxed(),
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
        }
    }
}
