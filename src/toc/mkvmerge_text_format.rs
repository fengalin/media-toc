use std::io::Write;

use super::{Chapter, Exporter};

const EXTENSION: &'static str = "txt";

pub struct MKVMergeTextFormat {
}

impl MKVMergeTextFormat {
    pub fn new_as_boxed() -> Box<Self> {
        Box::new(MKVMergeTextFormat{})
    }
}

impl Exporter for MKVMergeTextFormat {
    fn extension(&self) -> &'static str {
        &EXTENSION
    }

    fn write(&self, toc: &[Chapter], destination: &mut Write) {
        for (index, ref chapter) in toc.iter().enumerate() {
            let prefix = format!("CHAPTER{:02}", index + 1);
            destination.write_fmt(
                format_args!("{}={}\n",
                    prefix,
                    chapter.start.format_with_hours(),
                ))
                .expect("ExportController::export_btn clicked, failed to write in file");
            destination.write_fmt(format_args!("{}NAME={}\n", prefix, chapter.title()))
                .expect("ExportController::export_btn clicked, failed to write in file");
        }
    }
}
