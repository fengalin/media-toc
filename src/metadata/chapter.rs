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

    pub fn empty() -> Self {
        Chapter {
            id: String::new(),
            start: Timestamp::new(),
            end: Timestamp::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn set_title(&mut self, title: &str) {
        self.metadata
            .insert(super::METADATA_TITLE.to_owned(), title.to_owned());
    }

    pub fn get_title(&self) -> Option<&str> {
        self.metadata
            .get(super::METADATA_TITLE)
            .map(|string| string.as_str())
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
