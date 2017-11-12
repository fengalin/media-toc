use std::collections::HashMap;

use std::io::Read;

use super::Chapter;

pub trait Importer {
    fn read(&self,
        duration: u64,
        source: &mut Read,
        metadata: &mut HashMap<String, String>,
        chapters: &mut Vec<Chapter>,
    );
}
