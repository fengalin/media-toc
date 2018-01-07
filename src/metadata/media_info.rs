extern crate gstreamer as gst;

use std::collections::HashMap;

use metadata::Chapter;

pub struct MediaInfo {
    pub file_name: String,
    pub tags: gst::TagList,
    pub description: String,
    pub duration: u64,
    pub chapters: Vec<Chapter>,

    pub video_streams: HashMap<String, gst::Caps>,
    pub video_best: Option<String>,

    pub audio_streams: HashMap<String, gst::Caps>,
    pub audio_best: Option<String>,
}

impl MediaInfo {
    pub fn new() -> Self {
        MediaInfo {
            file_name: String::new(),
            tags: gst::TagList::new(),
            description: String::new(),
            duration: 0,
            chapters: Vec::new(),

            audio_streams: HashMap::new(),
            audio_best: None,

            video_streams: HashMap::new(),
            video_best: None,
        }
    }

    pub fn get_file_name(&self) -> &str {
        &self.file_name
    }

    pub fn get_artist(&self) -> Option<&str> {
        self.tags.get_index::<gst::tags::Artist>(0)
            .map(|value| value.get().unwrap())
            .or(
                self.tags.get_index::<gst::tags::AlbumArtist>(0)
                    .map(|value| value.get().unwrap())
            )
            .or(
                self.tags.get_index::<gst::tags::ArtistSortname>(0)
                    .map(|value| value.get().unwrap())
            )
            .or(
                self.tags.get_index::<gst::tags::AlbumArtistSortname>(0)
                    .map(|value| value.get().unwrap())
            )
    }

    pub fn get_title(&self) -> Option<&str> {
        self.tags.get_index::<gst::tags::Title>(0)
            .map(|value| value.get().unwrap())
    }

    pub fn get_image(&self, index: u32) -> Option<gst::Sample> {
        self.tags.get_index::<gst::tags::Image>(index)
            .map(|value| value.get().unwrap())
    }

    pub fn get_audio_codec(&self) -> Option<&str> {
        self.tags.get_index::<gst::tags::AudioCodec>(0)
            .map(|value| value.get().unwrap())
    }

    pub fn get_video_codec(&self) -> Option<&str> {
        self.tags.get_index::<gst::tags::VideoCodec>(0)
            .map(|value| value.get().unwrap())
    }

    pub fn get_container(&self) -> Option<&str> {
        // in case of an mp3 audio file, container comes as `ID3 label`
        // => bypass it
        if let Some(audio_codec) = self.get_audio_codec() {
            if self.get_video_codec().is_none() &&
                audio_codec.to_lowercase().find("mp3").is_some()
            {
                return None
            }
        }

        self.tags.get_index::<gst::tags::ContainerFormat>(0)
            .map(|value| value.get().unwrap())
    }
}
