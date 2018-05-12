use gettextrs::gettext;
use gstreamer as gst;

use std::collections::HashMap;

pub fn get_default_chapter_title() -> String {
    gettext("untitled")
}

#[derive(Clone)]
pub struct Stream {
    pub id: String,
    pub codec_printable: String,
    pub caps: gst::Caps,
    pub tags: Option<gst::TagList>,
    pub type_: gst::StreamType,
    pub must_export: bool,
}

impl Stream {
    fn new(stream: &gst::Stream) -> Self {
        let caps = stream.get_caps().unwrap();
        let tags = stream.get_tags();
        let type_ = stream.get_stream_type();

        let codec_printable = {
            let codec_printable = match tags.as_ref() {
                Some(tags) => {
                    let codec_printable = match type_ {
                        gst::StreamType::AUDIO => {
                            match tags.get_index::<gst::tags::AudioCodec>(0).as_ref() {
                                Some(codec) => codec.get(),
                                None => None,
                            }
                        }
                        gst::StreamType::VIDEO => {
                            match tags.get_index::<gst::tags::VideoCodec>(0).as_ref() {
                                Some(codec) => codec.get(),
                                None => None,
                            }
                        }
                        gst::StreamType::TEXT => {
                            match tags.get_index::<gst::tags::SubtitleCodec>(0).as_ref() {
                                Some(codec) => codec.get(),
                                None => None,
                            }
                        }
                        _ => panic!("Stream::new can't handle {:?}", type_),
                    };

                    match codec_printable {
                        Some(codec) => Some(codec),
                        None => match tags.get_index::<gst::tags::Codec>(0).as_ref() {
                            Some(codec) => codec.get(),
                            None => None,
                        },
                    }
                }
                None => None,
            };

            match codec_printable {
                Some(codec) => codec.to_string(),
                None => {
                    // codec in caps in the form "streamtype/x-codec"
                    let codec = caps.get_structure(0).unwrap().get_name();
                    let id_parts: Vec<&str> = codec.split('/').collect();
                    if id_parts.len() == 2 {
                        if id_parts[1].starts_with("x-") {
                            id_parts[1][2..].to_string()
                        } else {
                            id_parts[1].to_string()
                        }
                    } else {
                        codec.to_string()
                    }
                }
            }
        };

        Stream {
            id: stream.get_stream_id().unwrap(),
            codec_printable,
            caps,
            tags,
            type_,
            must_export: false,
        }
    }
}

#[derive(Default)]
pub struct Streams {
    pub audio: HashMap<String, Stream>,
    pub video: HashMap<String, Stream>,
    pub text: HashMap<String, Stream>,

    // FIXME: see if a self ref Stream could be used instead
    cur_audio_id: Option<String>,
    pub audio_changed: bool,
    cur_video_id: Option<String>,
    pub video_changed: bool,
    cur_text_id: Option<String>,
    pub text_changed: bool,
}

impl Streams {
    pub fn add_stream(&mut self, gst_stream: &gst::Stream) {
        let stream = Stream::new(gst_stream);
        match stream.type_ {
            gst::StreamType::AUDIO => {
                self.cur_audio_id.get_or_insert(stream.id.clone());
                self.audio.insert(stream.id.clone(), stream);
            }
            gst::StreamType::VIDEO => {
                self.cur_video_id.get_or_insert(stream.id.clone());
                self.video.insert(stream.id.clone(), stream);
            }
            gst::StreamType::TEXT => {
                self.cur_text_id.get_or_insert(stream.id.clone());
                self.text.insert(stream.id.clone(), stream);
            }
            _ => panic!("MediaInfo::add_stream can't handle {:?}", stream.type_),
        }
    }

    pub fn is_audio_selected(&self) -> bool {
        self.cur_audio_id.is_some()
    }

    pub fn is_video_selected(&self) -> bool {
        self.cur_video_id.is_some()
    }

    pub fn selected_audio(&self) -> Option<&Stream> {
        self.cur_audio_id.as_ref().map(|stream_id| {
            &self.audio[stream_id]
        })
    }

    pub fn selected_video(&self) -> Option<&Stream> {
        self.cur_video_id.as_ref().map(|stream_id| {
            &self.video[stream_id]
        })
    }

    pub fn selected_text(&self) -> Option<&Stream> {
        self.cur_text_id.as_ref().map(|stream_id| {
            &self.text[stream_id]
        })
    }

    // Returns the streams which changed
    pub fn select_streams(&mut self, ids: &[String]) {
        let mut is_audio_selected = false;
        let mut is_text_selected = false;
        let mut is_video_selected = false;

        for id in ids {
            if self.audio.contains_key(id) {
                is_audio_selected = true;
                self.audio_changed = self.selected_audio()
                    .map_or(true, |prev_stream| *id != prev_stream.id.as_str());
                self.cur_audio_id = Some(id.to_string());
            } else if self.text.contains_key(id) {
                is_text_selected = true;
                self.text_changed = self.selected_text()
                    .map_or(true, |prev_stream| *id != prev_stream.id.as_str());
                self.cur_text_id = Some(id.to_string());
            } else if self.video.contains_key(id) {
                is_video_selected = true;
                self.video_changed = self.selected_video()
                    .map_or(true, |prev_stream| *id != prev_stream.id.as_str());
                self.cur_video_id = Some(id.to_string());
            } else {
                panic!("MediaInfo::select_streams unknown stream id {}", id);
            }
        }

        if !is_audio_selected {
            self.audio_changed = self.cur_audio_id
                .take()
                .map_or(false, |_| true);
        }
        if !is_text_selected {
            self.text_changed = self.cur_text_id
                .take()
                .map_or(false, |_| true);
        }
        if !is_video_selected {
            self.video_changed = self.cur_video_id
                .take()
                .map_or(false, |_| true);
        }
    }
}

#[derive(Default)]
pub struct MediaInfo {
    pub file_name: String,
    pub tags: gst::TagList,
    pub toc: Option<gst::Toc>,
    pub chapter_count: Option<usize>,

    pub description: String,
    pub duration: u64,

    pub streams: Streams,
}

impl MediaInfo {
    pub fn new() -> Self {
        MediaInfo::default()
    }

    pub fn get_file_name(&self) -> &str {
        &self.file_name
    }

    pub fn get_artist(&self) -> Option<&str> {
        self.tags
            .get_index::<gst::tags::Artist>(0)
            .map(|value| value.get().unwrap())
            .or_else(|| {
                self.tags
                    .get_index::<gst::tags::AlbumArtist>(0)
                    .map(|value| value.get().unwrap())
            })
            .or_else(|| {
                self.tags
                    .get_index::<gst::tags::ArtistSortname>(0)
                    .map(|value| value.get().unwrap())
            })
            .or_else(|| {
                self.tags
                    .get_index::<gst::tags::AlbumArtistSortname>(0)
                    .map(|value| value.get().unwrap())
            })
    }

    pub fn get_title(&self) -> Option<&str> {
        self.tags
            .get_index::<gst::tags::Title>(0)
            .map(|value| value.get().unwrap())
    }

    pub fn get_image(&self, index: u32) -> Option<gst::Sample> {
        self.tags
            .get_index::<gst::tags::Image>(index)
            .map(|value| value.get().unwrap())
    }

    pub fn get_audio_codec(&self) -> Option<&str> {
        self.streams
            .selected_audio()
            .map(|stream| stream.codec_printable.as_str())
    }

    pub fn get_video_codec(&self) -> Option<&str> {
        self.streams
            .selected_video()
            .map(|stream| stream.codec_printable.as_str())
    }

    pub fn get_container(&self) -> Option<&str> {
        // in case of an mp3 audio file, container comes as `ID3 label`
        // => bypass it
        if let Some(audio_codec) = self.get_audio_codec() {
            if self.get_video_codec().is_none() && audio_codec.to_lowercase().find("mp3").is_some()
            {
                return None;
            }
        }

        self.tags
            .get_index::<gst::tags::ContainerFormat>(0)
            .map(|value| value.get().unwrap())
    }
}
