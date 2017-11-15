use std::collections::HashMap;

use std::io::{Read, Write};

use super::Chapter;

pub trait Importer {
    fn read(&self,
        duration: u64,
        source: &mut Read,
        metadata: &mut HashMap<String, String>,
        chapters: &mut Vec<Chapter>,
    );
}

pub trait Exporter {
    fn extension(&self) -> &'static str;
    fn write(&self,
        metadata: &HashMap<String, String>,
        chapters: &[Chapter],
        destination: &mut Write
    );
}
