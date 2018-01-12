extern crate gstreamer as gst;

use std::io::{Read, Write};

use super::{Chapter, MediaInfo};

pub trait Reader {
    fn read(&self, info: &MediaInfo, duration: u64, source: &mut Read, chapters: &mut Vec<Chapter>);
}

pub trait Writer {
    fn write(&self, info: &MediaInfo, chapters: &[Chapter], destination: &mut Write);
}

pub trait Exporter {
    fn export(&self, info: &MediaInfo, chapters: &[Chapter], destination: &gst::Element);
}
