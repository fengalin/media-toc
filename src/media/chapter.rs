use super::Timestamp;

use std::collections::HashMap;

pub struct Chapter {
    pub id: i32,
    pub start: Timestamp,
    pub end: Timestamp,
    pub metadata: HashMap<String, String>,
}

impl Chapter {
    pub fn new() -> Self {
        Chapter{
            id: 0,
            start: Timestamp::new(),
            end: Timestamp::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn set_id(&mut self, id: i32) {
        self.id = id;
    }

    pub fn set_start(&mut self, sec: i64, time_factor: f64) {
        self.start = Timestamp::from_sec_time_factor(sec, time_factor);
    }


    pub fn set_end(&mut self, sec: i64, time_factor: f64) {
        self.end = Timestamp::from_sec_time_factor(sec, time_factor);
    }

    pub fn title(&self) -> &str {
        match self.metadata.get("title") {
            Some(title) => &title,
            None => "",
        }
    }
}
