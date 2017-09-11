extern crate gstreamer as gst;

use std::collections::HashMap;

use super::{AlignedImage, Chapter};

pub struct MediaInfo {
    pub artist: String,
    pub title: String,
    pub description: String,
    pub duration: u64,
    pub chapters: Vec<Chapter>,

    pub thumbnail: Option<AlignedImage>,

    pub container: String,

    pub video_streams: HashMap<String, gst::Caps>,
    pub video_best: Option<String>,
    pub video_codec: String,

    pub audio_streams: HashMap<String, gst::Caps>,
    pub audio_best: Option<String>,
    pub audio_codec: String,
}

impl MediaInfo {
    pub fn new() -> Self {
        MediaInfo{
            artist: String::new(),
            title: String::new(),
            description: String::new(),
            duration: 0,
            chapters: Vec::new(),

            container: String::new(),

            thumbnail: None,

            video_streams: HashMap::new(),
            video_best: None,
            video_codec: String::new(),

            audio_streams: HashMap::new(),
            audio_best: None,
            audio_codec: String::new(),
        }
    }
}
