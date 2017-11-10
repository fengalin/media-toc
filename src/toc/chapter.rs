use super::Timestamp;

use std::clone::Clone;

use std::collections::HashMap;

pub struct Chapter {
    pub id: String,
    pub start: Timestamp,
    pub end: Timestamp,
    pub metadata: HashMap<String, String>,
}

impl Chapter {
    pub fn new(id: &str, title: &str, start: Timestamp, end: Timestamp) -> Self {
        let mut this = Chapter {
            id: id.to_owned(),
            start: start,
            end: end,
            metadata: HashMap::new(),
        };

        this.metadata.insert("title".to_owned(), title.to_string());

        this
    }

    pub fn get_title(&self) -> Option<&String> {
        self.metadata.get(super::METADATA_TITLE)
    }
}

impl Clone for Chapter {
    fn clone(&self) -> Self {
        Chapter {
            id: self.id.clone(),
            start: self.start,
            end: self.end,
            metadata: self.metadata.clone(),
        }
    }
}
