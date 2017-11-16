extern crate gstreamer as gst;

use std::collections::HashMap;

use std::io::{Read, Write};

use super::Chapter;

pub trait Reader {
    fn read(&self,
        duration: u64,
        source: &mut Read,
        metadata: &mut HashMap<String, String>,
        chapters: &mut Vec<Chapter>,
    );
}

pub trait Writer {
    fn write(&self,
        metadata: &HashMap<String, String>,
        chapters: &[Chapter],
        destination: &mut Write,
    );
}

pub trait Exporter {
    fn export(&self,
        metadata: &HashMap<String, String>,
        chapters: &[Chapter],
        destination: &gst::Element,
    );
}
