use std::boxed::Box;

use super::{CueSheetFormat, Exporter, Format, Importer, MKVMergeTextFormat};

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
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
            Format::CueSheet => unimplemented!("Importer for toc::Format::CueSheet"),
        }
    }

    pub fn get_exporter(format: Format) -> Box<Exporter> {
        match format {
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
            Format::CueSheet => CueSheetFormat::new_as_boxed(),
        }
    }
}
