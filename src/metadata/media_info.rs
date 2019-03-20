use gettextrs::gettext;
use gstreamer as gst;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{Format, MediaContent};

pub fn get_default_chapter_title() -> String {
    gettext("untitled")
}

#[macro_export]
macro_rules! get_artist (
    ($tags:expr) => (
        $tags.get_index::<gst::tags::Artist>(0)
            .map(|value| value.get().unwrap())
            .or_else(|| {
                $tags
                    .get_index::<gst::tags::AlbumArtist>(0)
                    .map(|value| value.get().unwrap())
            })
            .or_else(|| {
                $tags
                    .get_index::<gst::tags::ArtistSortname>(0)
                    .map(|value| value.get().unwrap())
            })
            .or_else(|| {
                $tags
                    .get_index::<gst::tags::AlbumArtistSortname>(0)
                    .map(|value| value.get().unwrap())
            })
            .map(|value| value.to_string())
    )
);

#[macro_export]
macro_rules! get_title (
    ($tags:expr) => (
        $tags.get_index::<gst::tags::Title>(0)
            .map(|value| value.get().unwrap().to_string())
    )
);

#[derive(Clone)]
pub struct Stream {
    pub id: Arc<str>,
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
            id: stream.get_stream_id().unwrap().as_str().into(),
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
    pub audio: HashMap<Arc<str>, Stream>,
    pub video: HashMap<Arc<str>, Stream>,
    pub text: HashMap<Arc<str>, Stream>,

    cur_audio_id: Option<Arc<str>>,
    pub audio_changed: bool,
    cur_video_id: Option<Arc<str>>,
    pub video_changed: bool,
    cur_text_id: Option<Arc<str>>,
    pub text_changed: bool,
}

impl Streams {
    pub fn add_stream(&mut self, gst_stream: &gst::Stream) {
        let stream = Stream::new(gst_stream);
        match stream.type_ {
            gst::StreamType::AUDIO => {
                self.cur_audio_id.get_or_insert(Arc::clone(&stream.id));
                self.audio.insert(stream.id.clone(), stream);
            }
            gst::StreamType::VIDEO => {
                self.cur_video_id.get_or_insert(Arc::clone(&stream.id));
                self.video.insert(stream.id.clone(), stream);
            }
            gst::StreamType::TEXT => {
                self.cur_text_id.get_or_insert(Arc::clone(&stream.id));
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
        self.cur_audio_id
            .as_ref()
            .map(|stream_id| &self.audio[stream_id])
    }

    pub fn selected_video(&self) -> Option<&Stream> {
        self.cur_video_id
            .as_ref()
            .map(|stream_id| &self.video[stream_id])
    }

    pub fn selected_text(&self) -> Option<&Stream> {
        self.cur_text_id
            .as_ref()
            .map(|stream_id| &self.text[stream_id])
    }

    pub fn get_audio_mut<S: AsRef<str>>(&mut self, id: S) -> Option<&mut Stream> {
        self.audio.get_mut(id.as_ref())
    }

    pub fn get_video_mut<S: AsRef<str>>(&mut self, id: S) -> Option<&mut Stream> {
        self.video.get_mut(id.as_ref())
    }

    pub fn get_text_mut<S: AsRef<str>>(&mut self, id: S) -> Option<&mut Stream> {
        self.text.get_mut(id.as_ref())
    }

    pub fn select_streams(&mut self, ids: &[Arc<str>]) {
        let mut is_audio_selected = false;
        let mut is_text_selected = false;
        let mut is_video_selected = false;

        for id in ids {
            if self.audio.contains_key(id) {
                is_audio_selected = true;
                self.audio_changed = self
                    .selected_audio()
                    .map_or(true, |prev_stream| *id != prev_stream.id);
                self.cur_audio_id = Some(Arc::clone(id));
            } else if self.text.contains_key(id) {
                is_text_selected = true;
                self.text_changed = self
                    .selected_text()
                    .map_or(true, |prev_stream| *id != prev_stream.id);
                self.cur_text_id = Some(Arc::clone(id));
            } else if self.video.contains_key(id) {
                is_video_selected = true;
                self.video_changed = self
                    .selected_video()
                    .map_or(true, |prev_stream| *id != prev_stream.id);
                self.cur_video_id = Some(Arc::clone(id));
            } else {
                panic!(
                    "MediaInfo::select_streams unknown stream id {}",
                    id.as_ref()
                );
            }
        }

        if !is_audio_selected {
            self.audio_changed = self.cur_audio_id.take().map_or(false, |_| true);
        }
        if !is_text_selected {
            self.text_changed = self.cur_text_id.take().map_or(false, |_| true);
        }
        if !is_video_selected {
            self.video_changed = self.cur_video_id.take().map_or(false, |_| true);
        }
    }
}

#[derive(Default)]
pub struct MediaInfo {
    pub name: String,
    pub file_name: String,
    pub path: PathBuf,
    pub content: MediaContent,
    pub tags: gst::TagList,
    pub toc: Option<gst::Toc>,
    pub chapter_count: Option<usize>,

    pub description: String,
    pub duration: u64,

    pub streams: Streams,
}

impl MediaInfo {
    pub fn new(path: &Path) -> Self {
        MediaInfo {
            name: path.file_stem().unwrap().to_str().unwrap().to_owned(),
            file_name: path.file_name().unwrap().to_str().unwrap().to_owned(),
            path: path.to_owned(),
            ..MediaInfo::default()
        }
    }

    pub fn add_stream(&mut self, gst_stream: &gst::Stream) {
        self.streams.add_stream(gst_stream);
        self.content.add_stream_type(gst_stream.get_stream_type());
    }

    pub fn add_tags(&mut self, tags: &gst::TagList) {
        self.tags = self.tags.merge(tags, gst::TagMergeMode::Keep);
    }

    pub fn fix_tags(&mut self) {
        let title = get_title!(self.tags).or_else(|| {
            let tags = if let Some(selected_audio) = self.streams.selected_audio() {
                Some(&selected_audio.tags)
            } else if let Some(selected_video) = self.streams.selected_video() {
                Some(&selected_video.tags)
            } else {
                None
            }?;

            match tags {
                Some(tags) => get_title!(tags),
                None => None,
            }
        });
        if let Some(title) = title {
            let tags = self.tags.get_mut().unwrap();
            tags.add::<gst::tags::Title>(&title.as_str(), gst::TagMergeMode::ReplaceAll);
        }

        let artist = get_artist!(self.tags).or_else(|| {
            let tags = if let Some(selected_audio) = self.streams.selected_audio() {
                Some(&selected_audio.tags)
            } else if let Some(selected_video) = self.streams.selected_video() {
                Some(&selected_video.tags)
            } else {
                None
            }?;

            match tags {
                Some(tags) => get_artist!(tags),
                None => None,
            }
        });
        if let Some(artist) = artist {
            let tags = self.tags.get_mut().unwrap();
            tags.add::<gst::tags::Artist>(&artist.as_str(), gst::TagMergeMode::ReplaceAll);
        }
    }

    pub fn get_file_name(&self) -> &str {
        &self.file_name
    }

    pub fn get_artist(&self) -> Option<String> {
        get_artist!(self.tags)
    }

    pub fn get_title(&self) -> Option<String> {
        get_title!(self.tags)
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

    pub fn get_stream_ids_to_export(&self, format: Format) -> (HashSet<String>, MediaContent) {
        let mut streams = HashSet::<String>::new();
        let mut content = MediaContent::Undefined;

        {
            if !format.is_audio_only() {
                for (stream_id, stream) in &self.streams.video {
                    if stream.must_export {
                        streams.insert(stream_id.to_string());
                        content.add_stream_type(gst::StreamType::VIDEO);
                    }
                }
                for (stream_id, stream) in &self.streams.text {
                    if stream.must_export {
                        streams.insert(stream_id.to_string());
                        content.add_stream_type(gst::StreamType::TEXT);
                    }
                }
            }

            for (stream_id, stream) in &self.streams.audio {
                if stream.must_export {
                    streams.insert(stream_id.to_string());
                    content.add_stream_type(gst::StreamType::AUDIO);
                }
            }
        }

        (streams, content)
    }
}
