use std::io::Write;

use std::collections::HashMap;

use super::{Chapter, Exporter};

static EXTENSION: &'static str = "toc.mkv";

pub struct MatroskaTocFormat {
}

impl MatroskaTocFormat {
    pub fn get_extension() -> &'static str {
        &EXTENSION
    }

    pub fn new_as_boxed() -> Box<Self> {
        Box::new(MatroskaTocFormat{})
    }
}

impl Exporter for MatroskaTocFormat {
    fn extension(&self) -> &'static str {
        MatroskaTocFormat::get_extension()
    }

    fn write(&self,
        _metadata: &HashMap<String, String>,
        _chapters: &[Chapter],
        _destination: &mut Write
    ) {
        unimplemented!("MatroskaToc::write");
    }
}
