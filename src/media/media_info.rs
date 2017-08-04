extern crate gstreamer as gst;

use std::clone::Clone;

use std::collections::HashMap;

use super::{Chapter, Timestamp};

pub struct MediaInfo {
    pub artist: String,
    pub title: String,
    pub duration: Timestamp,
    pub description: String,
    pub chapters: Vec<Chapter>,

    pub thumbnail: Option<Vec<u8>>,

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
            duration: Timestamp::new(),
            description: String::new(),
            chapters: Vec::new(),

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

impl Clone for MediaInfo {
    fn clone(&self) -> Self {
        // FIXME: there must be a better way
        MediaInfo {
            artist: self.artist.clone(),
            title: self.title.clone(),
            duration: self.duration.clone(),
            description: self.description.clone(),
            chapters: self.chapters.clone(),// TODO: find a way to avoid the copy

            thumbnail: self.thumbnail.clone(), // TODO: find a way to avoid the copy

            video_streams: self.video_streams.clone(),
            video_best: self.video_best.clone(),
            video_codec: self.video_codec.clone(),

            audio_streams: self.audio_streams.clone(),
            audio_best: self.audio_best.clone(),
            audio_codec: self.audio_codec.clone(),
        }
    }
}
