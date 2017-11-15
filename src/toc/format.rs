use std::collections::HashMap;

use std::io::Write;

use super::Chapter;

pub trait Exporter {
    fn extension(&self) -> &'static str;
    fn write(&self,
        metadata: &HashMap<String, String>,
        chapters: &[Chapter],
        destination: &mut Write
    );
}
