use std::collections::HashMap;

use std::io::Write;

use super::{Chapter, Exporter};

const EXTENSION: &str = "txt";

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

    fn write(&self,
        _metadata: &HashMap<String, String>,
        toc: &[Chapter],
        destination: &mut Write
    ) {
        for (index, chapter) in toc.iter().enumerate() {
            let prefix = format!("CHAPTER{:02}", index + 1);
            destination.write_fmt(
                format_args!("{}={}\n",
                    prefix,
                    chapter.start.format_with_hours(),
                ))
                .expect("MKVMergeTextFormat::write clicked, failed to write to file");
            if let Some(title) = chapter.get_title() {
                destination.write_fmt(format_args!("{}NAME={}\n", prefix, title))
                    .expect("MKVMergeTextFormat::write clicked, failed to write to file");
            }
        }
    }
}
