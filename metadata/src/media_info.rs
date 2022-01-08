use gettextrs::gettext;
use gst::{tags, StreamType, Tag, TagList, TagMergeMode};
use lazy_static::lazy_static;
use log::warn;

use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{Duration, Format, MediaContent};

#[derive(Debug)]
pub struct SelectStreamError(Arc<str>);

impl SelectStreamError {
    fn new(id: &Arc<str>) -> Self {
        SelectStreamError(Arc::clone(&id))
    }

    pub fn id(&self) -> &Arc<str> {
        &self.0
    }
}

impl fmt::Display for SelectStreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MediaInfo: unknown stream id {}", self.0)
    }
}
impl std::error::Error for SelectStreamError {}

pub fn default_chapter_title() -> String {
    gettext("untitled")
}

macro_rules! add_tag_names (
    ($($tag_type:path),+) => {
        {
            let mut tag_names = Vec::new();
            $(tag_names.push(<$tag_type>::tag_name());)+
            tag_names
        }
    };
);

lazy_static! {
    static ref TAGS_TO_SKIP_FOR_TRACK: Vec<&'static str> = {
        add_tag_names!(
            tags::Album,
            tags::AlbumSortname,
            tags::AlbumSortname,
            tags::AlbumArtist,
            tags::AlbumArtistSortname,
            tags::ApplicationName,
            tags::ApplicationData,
            tags::Artist,
            tags::ArtistSortname,
            tags::AudioCodec,
            tags::Codec,
            tags::ContainerFormat,
            tags::Duration,
            tags::Encoder,
            tags::EncoderVersion,
            tags::Image,
            tags::ImageOrientation,
            tags::PreviewImage,
            tags::SubtitleCodec,
            tags::Title,
            tags::TitleSortname,
            tags::TrackCount,
            tags::TrackNumber,
            tags::VideoCodec
        )
    };
}

macro_rules! add_first_tag (
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
        add_first_tag!($tags_src, $tags_dest, $tag_type, TagMergeMode::Append)
    };
);

macro_rules! add_stream_tags_if_empty (
    ($info:expr, $tags_dest:expr, $primary_tag:ty, $secondary_tag:ty) => {
        if $info.tags.get::<$primary_tag>().is_none() {
            let artist_stream_tags = $info.streams.tag_list::<$primary_tag>();
            add_first_tag!(artist_stream_tags, $tags_dest, $primary_tag);
            add_first_tag!(artist_stream_tags, $tags_dest, $secondary_tag, TagMergeMode::ReplaceAll);
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
        add_all_tags!($tags_src, $tags_dest, $tag_type, TagMergeMode::Append)
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
            .or_else(|| $info.streams.tag_list::<$tag_type>())
            .or_else(|| $info.tag_list::<$tag_type>())
    };
);

macro_rules! add_tags_for_chapter (
    ($info:expr, $chapter:expr, $tags_dest:expr, $primary_tag:ty, $secondary_tag:ty) => {
        let tags_list = get_tag_list_for_chapter!($info, $chapter, $primary_tag);
        add_first_tag!(tags_list, $tags_dest, $primary_tag, TagMergeMode::ReplaceAll);
        add_first_tag!(tags_list, $tags_dest, $secondary_tag, TagMergeMode::ReplaceAll);
    };
);

macro_rules! get_tag_for_display (
    ($info:expr, $primary_tag:ty, $secondary_tag:ty) => {
        #[allow(clippy::redundant_closure)]
        $info
            .tag_list::<$primary_tag>()
            .or_else(|| $info.tag_list::<$secondary_tag>())
            .or_else(|| {
                $info.streams
                    .tag_list::<$primary_tag>()
                    .or_else(|| $info.streams.tag_list::<$secondary_tag>())
            })
            .and_then(|tag_list| {
                tag_list
                    .get_index::<$primary_tag>(0)
                    .or_else(|| tag_list.get_index::<$secondary_tag>(0))
                    .and_then(|value| value.get().map(|ref_value| ref_value.to_owned()))
            })
    };
);

#[derive(Clone, Debug)]
pub struct Stream {
    pub id: Arc<str>,
    pub codec_printable: String,
    pub caps: gst::Caps,
    pub tags: TagList,
    pub type_: StreamType,
    pub must_export: bool,
}

impl Stream {
    fn new(stream: &gst::Stream) -> Self {
        let caps = stream.get_caps().unwrap();
        let tags = stream.get_tags().unwrap_or_else(TagList::new);
        let type_ = stream.get_stream_type();

        let codec_printable = match type_ {
            StreamType::AUDIO => tags.get_index::<tags::AudioCodec>(0),
            StreamType::VIDEO => tags.get_index::<tags::VideoCodec>(0),
            StreamType::TEXT => tags.get_index::<tags::SubtitleCodec>(0),
            _ => panic!("Stream::new can't handle {:?}", type_),
        }
        .or_else(|| tags.get_index::<tags::Codec>(0))
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
            str::to_string,
        );

        Stream {
            id: stream.get_stream_id().unwrap().as_str().into(),
            codec_printable,
            caps,
            tags,
            type_,
            must_export: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct StreamCollection {
    collection: HashMap<Arc<str>, Stream>,
}

impl StreamCollection {
    fn add_stream(&mut self, stream: Stream) {
        self.collection.insert(Arc::clone(&stream.id), stream);
    }

    pub fn len(&self) -> usize {
        self.collection.len()
    }

    pub fn get<S: AsRef<str>>(&self, id: S) -> Option<&Stream> {
        self.collection.get(id.as_ref())
    }

    pub fn get_mut<S: AsRef<str>>(&mut self, id: S) -> Option<&mut Stream> {
        self.collection.get_mut(id.as_ref())
    }

    pub fn contains<S: AsRef<str>>(&self, id: S) -> bool {
        self.collection.contains_key(id.as_ref())
    }

    pub fn sorted(&self) -> impl Iterator<Item = &'_ Stream> {
        SortedStreamCollectionIter::new(self)
    }
}

struct SortedStreamCollectionIter<'sc> {
    collection: &'sc StreamCollection,
    sorted_iter: std::vec::IntoIter<Arc<str>>,
}

impl<'sc> SortedStreamCollectionIter<'sc> {
    fn new(collection: &'sc StreamCollection) -> Self {
        let mut sorted_ids: Vec<Arc<str>> = collection.collection.keys().map(Arc::clone).collect();
        sorted_ids.sort();

        SortedStreamCollectionIter {
            collection,
            sorted_iter: sorted_ids.into_iter(),
        }
    }
}

impl<'sc> Iterator for SortedStreamCollectionIter<'sc> {
    type Item = &'sc Stream;

    fn next(&mut self) -> Option<Self::Item> {
        self.sorted_iter
            .next()
            .and_then(|id| self.collection.get(&id))
    }
}

#[derive(Debug, Default)]
pub struct Streams {
    pub audio: StreamCollection,
    pub video: StreamCollection,
    pub text: StreamCollection,

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
            StreamType::AUDIO => {
                self.cur_audio_id.get_or_insert(Arc::clone(&stream.id));
                self.audio.add_stream(stream);
            }
            StreamType::VIDEO => {
                self.cur_video_id.get_or_insert(Arc::clone(&stream.id));
                self.video.add_stream(stream);
            }
            StreamType::TEXT => {
                self.cur_text_id.get_or_insert(Arc::clone(&stream.id));
                self.text.add_stream(stream);
            }
            other => unimplemented!("{:?}", other),
        }
    }

    pub fn collection(&self, type_: StreamType) -> &StreamCollection {
        match type_ {
            StreamType::AUDIO => &self.audio,
            StreamType::VIDEO => &self.video,
            StreamType::TEXT => &self.text,
            other => unimplemented!("{:?}", other),
        }
    }

    pub fn collection_mut(&mut self, type_: StreamType) -> &mut StreamCollection {
        match type_ {
            StreamType::AUDIO => &mut self.audio,
            StreamType::VIDEO => &mut self.video,
            StreamType::TEXT => &mut self.text,
            other => unimplemented!("{:?}", other),
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
            .and_then(|stream_id| self.audio.get(stream_id))
    }

    pub fn selected_video(&self) -> Option<&Stream> {
        self.cur_video_id
            .as_ref()
            .and_then(|stream_id| self.video.get(stream_id))
    }

    pub fn selected_text(&self) -> Option<&Stream> {
        self.cur_text_id
            .as_ref()
            .and_then(|stream_id| self.text.get(stream_id))
    }

    pub fn select_streams(&mut self, ids: &[Arc<str>]) -> Result<(), SelectStreamError> {
        let mut is_audio_selected = false;
        let mut is_text_selected = false;
        let mut is_video_selected = false;

        for id in ids {
            if self.audio.contains(id) {
                is_audio_selected = true;
                self.audio_changed = self
                    .selected_audio()
                    .map_or(true, |prev_stream| *id != prev_stream.id);
                self.cur_audio_id = Some(Arc::clone(id));
            } else if self.text.contains(id) {
                is_text_selected = true;
                self.text_changed = self
                    .selected_text()
                    .map_or(true, |prev_stream| *id != prev_stream.id);
                self.cur_text_id = Some(Arc::clone(id));
            } else if self.video.contains(id) {
                is_video_selected = true;
                self.video_changed = self
                    .selected_video()
                    .map_or(true, |prev_stream| *id != prev_stream.id);
                self.cur_video_id = Some(Arc::clone(id));
            } else {
                return Err(SelectStreamError::new(id));
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

        Ok(())
    }

    pub fn audio_codec(&self) -> Option<&str> {
        self.selected_audio()
            .map(|stream| stream.codec_printable.as_str())
    }

    pub fn video_codec(&self) -> Option<&str> {
        self.selected_video()
            .map(|stream| stream.codec_printable.as_str())
    }

    pub fn ids_to_export(&self, format: Format) -> (HashSet<String>, MediaContent) {
        let mut streams = HashSet::<String>::new();
        let mut content = MediaContent::Undefined;

        {
            if !format.is_audio_only() {
                for (stream_id, stream) in &self.video.collection {
                    if stream.must_export {
                        streams.insert(stream_id.to_string());
                        content.add_stream_type(StreamType::VIDEO);
                    }
                }
                // FIXME: discard text stream export for now as it hangs the export
                // (see https://github.com/fengalin/media-toc/issues/136)
                /*
                for (stream_id, stream) in &self.text.collection {
                    if stream.must_export {
                        streams.insert(stream_id.to_string());
                        content.add_stream_type(StreamType::TEXT);
                    }
                }
                */
            }

            for (stream_id, stream) in &self.audio.collection {
                if stream.must_export {
                    streams.insert(stream_id.to_string());
                    content.add_stream_type(StreamType::AUDIO);
                }
            }
        }

        (streams, content)
    }

    fn tag_list<'a, T: Tag<'a>>(&self) -> Option<TagList> {
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
    pub tags: TagList,
    pub toc: Option<gst::Toc>,
    pub chapter_count: Option<usize>,

    pub description: String,
    pub duration: Duration,

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

    pub fn add_tags(&mut self, tags: &TagList) {
        self.tags = self.tags.merge(tags, TagMergeMode::Keep);
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    fn tag_list<'a, T: Tag<'a>>(&self) -> Option<TagList> {
        if self.tags.get_size::<T>() > 0 {
            Some(self.tags.clone())
        } else {
            None
        }
    }

    /// Fill missing tags for global scope
    #[allow(clippy::cognitive_complexity)]
    pub fn fixed_tags(&self) -> TagList {
        let mut tags = TagList::new();
        {
            let tags = tags.get_mut().unwrap();
            tags.insert(&self.tags, TagMergeMode::ReplaceAll);
            tags.add::<tags::ApplicationName>(&"media-toc", TagMergeMode::ReplaceAll);

            // Attempt to fill missing global tags with current stream tags
            add_stream_tags_if_empty!(self, tags, tags::Artist, tags::ArtistSortname);
            add_stream_tags_if_empty!(self, tags, tags::Album, tags::AlbumSortname);
            add_stream_tags_if_empty!(self, tags, tags::AlbumArtist, tags::AlbumArtistSortname);
            add_stream_tags_if_empty!(self, tags, tags::Title, tags::TitleSortname);

            if self.tags.get::<tags::Image>().is_none() {
                let image_stream_tags = self.streams.tag_list::<tags::Image>();
                add_all_tags!(image_stream_tags, tags, tags::Image);
                add_all_tags!(image_stream_tags, tags, tags::ImageOrientation);
            }

            if self.tags.get::<tags::PreviewImage>().is_none() {
                let image_stream_tags = self.streams.tag_list::<tags::PreviewImage>();
                add_all_tags!(image_stream_tags, tags, tags::PreviewImage);
            }
        }

        tags
    }

    #[allow(clippy::cognitive_complexity)]
    pub fn chapter_with_track_tags(
        &self,
        chapter: &gst::TocEntry,
        track_number: usize,
    ) -> gst::TocEntry {
        let mut tags = TagList::new();
        {
            let tags = tags.get_mut().unwrap();

            // Select tags suitable for a track
            for (tag_name, tag_iter) in self.tags.iter_generic() {
                if TAGS_TO_SKIP_FOR_TRACK
                    .iter()
                    .all(|&tag_to_skip| tag_to_skip != tag_name)
                {
                    // can add tag
                    for tag_value in tag_iter {
                        if tags
                            .add_value(tag_name, tag_value, TagMergeMode::Append)
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
                if chapter_tags.get_size::<tags::Title>() > 0 {
                    Some(chapter_tags)
                } else {
                    None
                }
            });
            if title_tags.is_some() {
                add_first_tag!(title_tags, tags, tags::Title);
                add_first_tag!(title_tags, tags, tags::TitleSortname);
            } else {
                tags.add::<tags::Title>(&default_chapter_title().as_str(), TagMergeMode::Append);
            }

            // Use the media Title as the track Album (which folds back to Album)
            if let Some(track_album) = self.media_title() {
                tags.add::<tags::Album>(&track_album.as_str(), TagMergeMode::Append);
            }
            if let Some(track_album) = self.media_title_sortname() {
                tags.add::<tags::AlbumSortname>(&track_album.as_str(), TagMergeMode::Append);
            }

            add_tags_for_chapter!(self, chapter, tags, tags::Artist, tags::ArtistSortname);
            add_tags_for_chapter!(
                self,
                chapter,
                tags,
                tags::AlbumArtist,
                tags::AlbumArtistSortname
            );

            let image_tags = get_tag_list_for_chapter!(self, chapter, tags::Image);
            add_all_tags!(image_tags, tags, tags::Image);
            add_all_tags!(image_tags, tags, tags::ImageOrientation);

            let preview_image_tags = get_tag_list_for_chapter!(self, chapter, tags::PreviewImage);
            add_all_tags!(preview_image_tags, tags, tags::PreviewImage);

            let (start, end) = chapter.get_start_stop_times().unwrap();

            tags.add::<tags::TrackNumber>(&(track_number as u32), TagMergeMode::ReplaceAll);
            tags.add::<tags::TrackCount>(&(chapter_count as u32), TagMergeMode::ReplaceAll);
            tags.add::<tags::Duration>(
                &gst::ClockTime::from_nseconds((end - start) as u64),
                TagMergeMode::ReplaceAll,
            );
            tags.add::<tags::ApplicationName>(&"media-toc", TagMergeMode::ReplaceAll);
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

    pub fn media_artist(&self) -> Option<String> {
        get_tag_for_display!(self, tags::Artist, tags::AlbumArtist)
    }

    pub fn media_artist_sortname(&self) -> Option<String> {
        get_tag_for_display!(self, tags::ArtistSortname, tags::AlbumArtistSortname)
    }

    pub fn media_title(&self) -> Option<String> {
        get_tag_for_display!(self, tags::Title, tags::Album)
    }

    pub fn media_title_sortname(&self) -> Option<String> {
        get_tag_for_display!(self, tags::TitleSortname, tags::AlbumSortname)
    }

    pub fn media_image(&self) -> Option<gst::Sample> {
        get_tag_for_display!(self, tags::Image, tags::PreviewImage)
    }

    pub fn container(&self) -> Option<&str> {
        // in case of an mp3 audio file, container comes as `ID3 label`
        // => bypass it
        if let Some(audio_codec) = self.streams.audio_codec() {
            if self.streams.video_codec().is_none()
                && audio_codec.to_lowercase().find("mp3").is_some()
            {
                return None;
            }
        }

        self.tags
            .get_index::<tags::ContainerFormat>(0)
            .and_then(glib::value::TypedValue::get)
    }
}
