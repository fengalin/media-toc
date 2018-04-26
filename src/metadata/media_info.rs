use gettextrs::gettext;
use gstreamer as gst;

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
        }
    }
}

#[derive(Default)]
pub struct Streams {
    pub audio: Vec<Stream>,
    pub video: Vec<Stream>,
    pub text: Vec<Stream>,

    pub audio_selected: Option<Stream>,
    pub video_selected: Option<Stream>,
    pub text_selected: Option<Stream>,
}

impl Streams {
    pub fn add_stream(&mut self, gst_stream: &gst::Stream) {
        let stream = Stream::new(gst_stream);
        match stream.type_ {
            gst::StreamType::AUDIO => {
                self.audio_selected.get_or_insert(stream.clone());
                self.audio.push(stream);
            }
            gst::StreamType::VIDEO => {
                self.video_selected.get_or_insert(stream.clone());
                self.video.push(stream);
            }
            gst::StreamType::TEXT => {
                self.text_selected.get_or_insert(stream.clone());
                self.text.push(stream);
            }
            _ => panic!("MediaInfo::add_stream can't handle {:?}", stream.type_),
        }
    }

    // Returns the streams which changed
    pub fn select_streams(&mut self, ids: &[String]) {
        // FIXME: handle unselected stream types (e.g. for subtitles)
        self.reset_selected();

        for id in ids {
            match self.video.iter().find(|s| &s.id == id) {
                Some(stream) => self.video_selected = Some(stream.clone()),
                None => match self.audio.iter().find(|s| &s.id == id) {
                    Some(stream) => self.audio_selected = Some(stream.clone()),
                    None => match self.text.iter().find(|s| &s.id == id) {
                        Some(stream) => self.text_selected = Some(stream.clone()),
                        None => panic!("MediaInfo::select_streams unknown stream id {}", id),
                    },
                },
            }
        }
    }

    pub fn reset_selected(&mut self) {
        self.audio_selected = None;
        self.video_selected = None;
        self.text_selected = None;
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
            .audio_selected
            .as_ref()
            .map(|stream| stream.codec_printable.as_str())
    }

    pub fn get_video_codec(&self) -> Option<&str> {
        self.streams
            .video_selected
            .as_ref()
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
