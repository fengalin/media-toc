use gstreamer as gst;

use std::io::{Read, Write};

use super::MediaInfo;

pub trait Reader {
    fn read(&self, info: &MediaInfo, source: &mut Read) -> Option<gst::Toc>;
}

pub trait Writer {
    fn write(&self, info: &MediaInfo, destination: &mut Write);
}

pub trait Exporter {
    fn export(&self, info: &MediaInfo, destination: &gst::Element);
}
