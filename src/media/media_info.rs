extern crate gstreamer as gst;

use std::collections::HashMap;

use toc;
use toc::Chapter;

use super::AlignedImage;

pub struct MediaInfo {
    pub metadata: HashMap<String, String>,
    pub description: String,
    pub duration: u64,
    pub chapters: Vec<Chapter>,

    pub thumbnail: Option<AlignedImage>,

    pub video_streams: HashMap<String, gst::Caps>,
    pub video_best: Option<String>,

    pub audio_streams: HashMap<String, gst::Caps>,
    pub audio_best: Option<String>,
}

impl MediaInfo {
    pub fn new() -> Self {
        MediaInfo {
            metadata: HashMap::new(),
            description: String::new(),
            duration: 0,
            chapters: Vec::new(),

            thumbnail: None,

            audio_streams: HashMap::new(),
            audio_best: None,

            video_streams: HashMap::new(),
            video_best: None,
        }
    }

    pub fn get_artist(&self) -> Option<&String> {
        self.metadata.get(toc::METADATA_ARTIST)
    }

    pub fn get_title(&self) -> Option<&String> {
        self.metadata.get(toc::METADATA_TITLE)
    }

    pub fn get_audio_codec(&self) -> Option<&String> {
        self.metadata.get(toc::METADATA_AUDIO_CODEC)
    }

    pub fn get_video_codec(&self) -> Option<&String> {
        self.metadata.get(toc::METADATA_VIDEO_CODEC)
    }

    pub fn get_container(&self) -> Option<&String> {
        self.metadata.get(toc::METADATA_CONTAINER)
    }

    // Fix specific cases
    pub fn fix(&mut self) {
        let audio_codec = self.get_audio_codec().cloned();
        if let Some(audio_codec) = audio_codec {
            if self.get_video_codec().is_none() &&
                audio_codec.to_lowercase().find("mp3").is_some()
            {
                // in case of an mp3 audio file, container comes as `ID3 label`
                self.metadata.remove(toc::METADATA_CONTAINER);
            }
        }
    }
}
