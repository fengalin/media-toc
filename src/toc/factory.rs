use std::boxed::Box;

use super::{Exporter, Format, MKVMergeTextFormat};

pub struct Factory {
}

impl Factory {
    pub fn get_exporter(format: Format) -> Box<Exporter> {
        match format {
            Format::MKVMergeText => MKVMergeTextFormat::new_as_boxed(),
        }
    }
}
