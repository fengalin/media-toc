use std::boxed::Box;

use super::{CueSheetFormat, Exporter, Format, MKVMergeTextFormat};

pub struct Factory {
}

impl Factory {
    pub fn get_exporter(format: Format) -> Box<Exporter> {
        match format {
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
            Format::CueSheet => CueSheetFormat::new_as_boxed(),
        }
    }
}
