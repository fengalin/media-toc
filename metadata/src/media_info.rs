use gettextrs::gettext;
use gst::tags::*;
use gstreamer as gst;
use lazy_static::lazy_static;
use log::warn;

use std::{
    borrow::ToOwned,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    string::ToString,
    sync::Arc,
};

use super::{Format, MediaContent};

pub fn get_default_chapter_title() -> String {
    gettext("untitled")
}

macro_rules! add_tag_names (
    ($($tag_type:ident),+) => {
        {
            let mut tag_names = Vec::new();
            $(tag_names.push($tag_type::tag_name());)+
            tag_names
        }
    };
);

lazy_static! {
    static ref TAGS_TO_SKIP_FOR_TRACK: Vec<&'static str> = {
        add_tag_names!(
            Album,
            AlbumSortname,
            AlbumSortname,
            AlbumArtist,
            AlbumArtistSortname,
            ApplicationName,
            ApplicationData,
            Artist,
            ArtistSortname,
            AudioCodec,
            Codec,
            ContainerFormat,
            Duration,
            Encoder,
            EncoderVersion,
            Image,
            ImageOrientation,
            PreviewImage,
            SubtitleCodec,
            Title,
            TitleSortname,
            TrackCount,
            TrackNumber,
            VideoCodec
        )
    };
}

macro_rules! add_first_str_tag (
    ($tags_src:expr, $tags_dest:expr, $tag_type:ty, $merge_mode:expr) => {
        if let Some(tags_src) = &$tags_src {
            if let Some(tag) = tags_src.get_index::<$tag_type>(0) {
                if let Some(value) = tag.get() {
                    $tags_dest.add::<$tag_type>(&value, $merge_mode);
                }
            }
        }
    };
    ($tags_src:expr, $tags_dest:expr, $tag_type:ty) => {
        add_first_str_tag!($tags_src, $tags_dest, $tag_type, gst::TagMergeMode::Append)
    };
);

macro_rules! add_stream_str_tags_if_empty (
    ($info:expr, $tags_dest:expr, $primary_tag:ty, $secondary_tag:ty) => {
        if $info.tags.get::<$primary_tag>().is_none() {
            let artist_stream_tags = $info.streams.get_tag_list::<$primary_tag>();
            add_first_str_tag!(artist_stream_tags, $tags_dest, $primary_tag);
            add_first_str_tag!(artist_stream_tags, $tags_dest, $secondary_tag, gst::TagMergeMode::ReplaceAll);
        }
    };
);

macro_rules! add_all_tags (
    ($tags_src:expr, $tags_dest:expr, $tag_type:ty, $merge_mode:expr) => {
        if let Some(tags_src) = &$tags_src {
            let tags_iter = tags_src.iter_tag::<$tag_type>();
            for tag in tags_iter {
                if let Some(value) = tag.get() {
                    $tags_dest.add::<$tag_type>(&value, $merge_mode);
                }
            }
        }
    };
    ($tags_src:expr, $tags_dest:expr, $tag_type:ty) => {
        add_all_tags!($tags_src, $tags_dest, $tag_type, gst::TagMergeMode::Append)
    };
);

macro_rules! get_tag_list_for_chapter (
    ($info:expr, $chapter:expr, $tag_type:ty) => {
        $chapter
            .get_tags()
            .and_then(|chapter_tags| {
                if chapter_tags.get_size::<$tag_type>() > 0 {
                    Some(chapter_tags.clone())
                } else {
                    None
                }
            })
            .or_else(|| $info.streams.get_tag_list::<$tag_type>())
            .or_else(|| $info.get_tag_list::<$tag_type>())
    };
);

macro_rules! add_str_tags_for_chapter (
    ($info:expr, $chapter:expr, $tags_dest:expr, $primary_tag:ty, $secondary_tag:ty) => {
        let tags_list = get_tag_list_for_chapter!($info, $chapter, $primary_tag);
        add_first_str_tag!(tags_list, $tags_dest, $primary_tag, gst::TagMergeMode::ReplaceAll);
        add_first_str_tag!(tags_list, $tags_dest, $secondary_tag, gst::TagMergeMode::ReplaceAll);
    };
);

macro_rules! get_tag_for_display (
    ($info:expr, $primary_tag:ty, $secondary_tag:ty) => {
        #[allow(clippy::redundant_closure)]
        $info
            .get_tag_list::<$primary_tag>()
            .or_else(|| $info.get_tag_list::<$secondary_tag>())
            .or_else(|| {
                $info.streams
                    .get_tag_list::<$primary_tag>()
                    .or_else(|| $info.streams.get_tag_list::<$secondary_tag>())
            })
            .and_then(|tag_list| {
                tag_list
                    .get_index::<$primary_tag>(0)
                    .or_else(|| tag_list.get_index::<$secondary_tag>(0))
                    .and_then(|value| value.get().map(|ref_value| ref_value.to_owned()))
            })
    };
);

#[derive(Clone)]
pub struct Stream {
    pub id: Arc<str>,
    pub codec_printable: String,
    pub caps: gst::Caps,
    pub tags: gst::TagList,
    pub type_: gst::StreamType,
    pub must_export: bool,
}

impl Stream {
    fn new(stream: &gst::Stream) -> Self {
        let caps = stream.get_caps().unwrap();
        let tags = stream
            .get_tags()
            .unwrap_or_else(gst::TagList::new);
        let type_ = stream.get_stream_type();

        let codec_printable = match type_ {
            gst::StreamType::AUDIO => tags.get_index::<AudioCodec>(0),
            gst::StreamType::VIDEO => tags.get_index::<VideoCodec>(0),
            gst::StreamType::TEXT => tags.get_index::<SubtitleCodec>(0),
            _ => panic!("Stream::new can't handle {:?}", type_),
        }
        .or_else(|| tags.get_index::<Codec>(0))
        .and_then(glib::value::TypedValue::get)
        .map_or_else(
            || {
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
            },
            ToString::to_string,
        );

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

    pub fn get_audio_codec(&self) -> Option<&str> {
        self.selected_audio()
            .map(|stream| stream.codec_printable.as_str())
    }

    pub fn get_video_codec(&self) -> Option<&str> {
        self.selected_video()
            .map(|stream| stream.codec_printable.as_str())
    }

    pub fn get_ids_to_export(&self, format: Format) -> (HashSet<String>, MediaContent) {
        let mut streams = HashSet::<String>::new();
        let mut content = MediaContent::Undefined;

        {
            if !format.is_audio_only() {
                for (stream_id, stream) in &self.video {
                    if stream.must_export {
                        streams.insert(stream_id.to_string());
                        content.add_stream_type(gst::StreamType::VIDEO);
                    }
                }
                for (stream_id, stream) in &self.text {
                    if stream.must_export {
                        streams.insert(stream_id.to_string());
                        content.add_stream_type(gst::StreamType::TEXT);
                    }
                }
            }

            for (stream_id, stream) in &self.audio {
                if stream.must_export {
                    streams.insert(stream_id.to_string());
                    content.add_stream_type(gst::StreamType::AUDIO);
                }
            }
        }

        (streams, content)
    }

    fn get_tag_list<'a, T: Tag<'a>>(&self) -> Option<gst::TagList> {
        self.selected_audio()
            .and_then(|selected_audio| {
                if selected_audio.tags.get_size::<T>() > 0 {
                    Some(selected_audio.tags.clone())
                } else {
                    None
                }
            })
            .or_else(|| {
                self.selected_video().and_then(|selected_video| {
                    if selected_video.tags.get_size::<T>() > 0 {
                        Some(selected_video.tags.clone())
                    } else {
                        None
                    }
                })
            })
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

    pub fn get_file_name(&self) -> &str {
        &self.file_name
    }

    fn get_tag_list<'a, T: Tag<'a>>(&self) -> Option<gst::TagList> {
        if self.tags.get_size::<T>() > 0 {
            Some(self.tags.clone())
        } else {
            None
        }
    }

    /// Fill missing tags for global scope
    #[allow(clippy::cyclomatic_complexity)]
    pub fn get_fixed_tags(&self) -> gst::TagList {
        let mut tags = gst::TagList::new();
        {
            let tags = tags.get_mut().unwrap();
            tags.insert(&self.tags, gst::TagMergeMode::ReplaceAll);
            tags.add::<ApplicationName>(&"media-toc", gst::TagMergeMode::ReplaceAll);

            // Attempt to fill missing global tags with current stream tags
            add_stream_str_tags_if_empty!(self, tags, Artist, ArtistSortname);
            add_stream_str_tags_if_empty!(self, tags, Album, AlbumSortname);
            add_stream_str_tags_if_empty!(self, tags, AlbumArtist, AlbumArtistSortname);
            add_stream_str_tags_if_empty!(self, tags, Title, TitleSortname);

            if self.tags.get::<Image>().is_none() {
                let image_stream_tags = self.streams.get_tag_list::<Image>();
                add_all_tags!(image_stream_tags, tags, Image);
                add_all_tags!(image_stream_tags, tags, ImageOrientation);
            }

            if self.tags.get::<PreviewImage>().is_none() {
                let image_stream_tags = self.streams.get_tag_list::<PreviewImage>();
                add_all_tags!(image_stream_tags, tags, PreviewImage);
            }
        }

        tags
    }

    #[allow(clippy::cyclomatic_complexity)]
    pub fn get_chapter_with_track_tags(
        &self,
        chapter: &gst::TocEntry,
        track_number: usize,
    ) -> gst::TocEntry {
        let mut tags = gst::TagList::new();
        {
            let tags = tags.get_mut().unwrap();

            // Select tags suitable for a track
            for (tag_name, tag_iter) in self.tags.iter_generic() {
                if TAGS_TO_SKIP_FOR_TRACK
                    .iter()
                    .find(|&&tag_to_skip| tag_to_skip == tag_name)
                    .is_none()
                {
                    // can add tag
                    for tag_value in tag_iter {
                        if tags
                            .add_generic(tag_name, tag_value, gst::TagMergeMode::Append)
                            .is_err()
                        {
                            warn!(
                                "{}",
                                gettext("couldn't add tag {tag_name}").replacen(
                                    "{tag_name}",
                                    tag_name,
                                    1
                                )
                            );
                        }
                    }
                }
            }
            let chapter_count = self.chapter_count.unwrap_or(1);

            // FIXME: add Sortname variantes

            // Add track specific tags
            // Title is special as we don't fallback to the global title but give a default
            let title_tags = chapter.get_tags().and_then(|chapter_tags| {
                if chapter_tags.get_size::<Title>() > 0 {
                    Some(chapter_tags.clone())
                } else {
                    None
                }
            });
            if title_tags.is_some() {
                add_first_str_tag!(title_tags, tags, Title);
                add_first_str_tag!(title_tags, tags, TitleSortname);
            } else {
                tags.add::<Title>(
                    &get_default_chapter_title().as_str(),
                    gst::TagMergeMode::Append,
                );
            }

            // Use the media Title as the track Album (which folds back to Album)
            if let Some(track_album) = self.get_media_title() {
                tags.add::<Album>(&track_album.as_str(), gst::TagMergeMode::Append);
            }
            if let Some(track_album) = self.get_media_title_sortname() {
                tags.add::<AlbumSortname>(&track_album.as_str(), gst::TagMergeMode::Append);
            }

            add_str_tags_for_chapter!(self, chapter, tags, Artist, ArtistSortname);
            add_str_tags_for_chapter!(self, chapter, tags, AlbumArtist, AlbumArtistSortname);

            let image_tags = get_tag_list_for_chapter!(self, chapter, Image);
            add_all_tags!(image_tags, tags, Image);
            add_all_tags!(image_tags, tags, ImageOrientation);

            let preview_image_tags = get_tag_list_for_chapter!(self, chapter, PreviewImage);
            add_all_tags!(preview_image_tags, tags, PreviewImage);

            let (start, end) = chapter.get_start_stop_times().unwrap();

            tags.add::<TrackNumber>(&(track_number as u32), gst::TagMergeMode::ReplaceAll);
            tags.add::<TrackCount>(&(chapter_count as u32), gst::TagMergeMode::ReplaceAll);
            tags.add::<Duration>(
                &gst::ClockTime::from_nseconds((end - start) as u64),
                gst::TagMergeMode::ReplaceAll,
            );
            tags.add::<ApplicationName>(&"media-toc", gst::TagMergeMode::ReplaceAll);
        }

        let mut track_chapter = gst::TocEntry::new(chapter.get_entry_type(), chapter.get_uid());
        {
            let track_chapter = track_chapter.get_mut().unwrap();
            let (start, end) = chapter.get_start_stop_times().unwrap();
            track_chapter.set_start_stop_times(start, end);
            track_chapter.set_tags(tags);
        }

        track_chapter
    }

    pub fn get_media_artist(&self) -> Option<String> {
        get_tag_for_display!(self, Artist, AlbumArtist)
    }

    pub fn get_media_artist_sortname(&self) -> Option<String> {
        get_tag_for_display!(self, ArtistSortname, AlbumArtistSortname)
    }

    pub fn get_media_title(&self) -> Option<String> {
        get_tag_for_display!(self, Title, Album)
    }

    pub fn get_media_title_sortname(&self) -> Option<String> {
        get_tag_for_display!(self, TitleSortname, AlbumSortname)
    }

    pub fn get_media_image(&self) -> Option<gst::Sample> {
        get_tag_for_display!(self, Image, PreviewImage)
    }

    pub fn get_container(&self) -> Option<&str> {
        // in case of an mp3 audio file, container comes as `ID3 label`
        // => bypass it
        if let Some(audio_codec) = self.streams.get_audio_codec() {
            if self.streams.get_video_codec().is_none()
                && audio_codec.to_lowercase().find("mp3").is_some()
            {
                return None;
            }
        }

        self.tags
            .get_index::<ContainerFormat>(0)
            .and_then(glib::value::TypedValue::get)
    }
}
