use std::io::{Read, Write};

use super::MediaInfo;

pub trait Reader {
    fn read(&self, info: &MediaInfo, source: &mut dyn Read) -> Result<Option<gst::Toc>, String>;
}

pub trait Writer {
    fn write(&self, info: &MediaInfo, destination: &mut dyn Write) -> Result<(), String>;
}

pub trait Exporter {
    fn export(&self, info: &MediaInfo, destination: &gst::Element);
}
