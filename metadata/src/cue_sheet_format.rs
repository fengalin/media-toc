use gettextrs::gettext;
use gstreamer as gst;

use log::error;

use std::{io::Write, string::ToString};

use super::{get_default_chapter_title, MediaInfo, Timestamp4Humans, TocVisitor, Writer};

static EXTENSION: &str = "cue";

pub struct CueSheetFormat {}

impl CueSheetFormat {
    pub fn get_extension() -> &'static str {
        EXTENSION
    }

    pub fn new_as_boxed() -> Box<Self> {
        Box::new(CueSheetFormat {})
    }
}

macro_rules! write_fmt(
    ($dest:ident, $fmt:expr, $( $item:expr ),*) => {
        $dest.write_fmt(format_args!($fmt, $( $item ),*)).map_err(|_| {
            let msg = gettext("Failed to write Cue Sheet file");
            error!("{}", msg);
            msg
        })?;
    };
);

impl Writer for CueSheetFormat {
    fn write(&self, info: &MediaInfo, destination: &mut dyn Write) -> Result<(), String> {
        if info.toc.is_none() {
            let msg = gettext("The table of contents is empty");
            error!("{}", msg);
            return Err(msg);
        }

        let media_title = info.get_media_title();
        if let Some(title) = &media_title {
            write_fmt!(destination, "TITLE \"{}\"\n", title);
        }

        let media_artist = info.get_media_artist();
        if let Some(artist) = &media_artist {
            write_fmt!(destination, "PERFORMER \"{}\"\n", artist);
        }

        let audio_codec = info
            .streams
            .get_audio_codec()
            .map_or("WAVE", |audio_codec| {
                if audio_codec.to_lowercase().find("mp3").is_some() {
                    "MP3"
                } else if audio_codec.to_lowercase().find("aiff").is_some() {
                    "AIFF"
                } else {
                    "WAVE"
                }
            });
        write_fmt!(
            destination,
            "FILE \"{}\" {}\n",
            info.get_file_name(),
            audio_codec
        );

        let mut index = 0;
        let mut toc_visitor = TocVisitor::new(info.toc.as_ref().unwrap());
        while let Some(chapter) = toc_visitor.next_chapter() {
            index += 1;
            // FIXME: are there other TRACK types than AUDIO?
            write_fmt!(destination, "  TRACK{:02} AUDIO\n", index);

            let title = chapter
                .get_tags()
                .and_then(|tags| {
                    tags.get::<gst::tags::Title>()
                        .and_then(|value| value.get().map(ToString::to_string))
                })
                .or_else(|| media_title.clone())
                .unwrap_or_else(get_default_chapter_title);
            write_fmt!(destination, "    TITLE \"{}\"\n", &title);

            let artist = chapter
                .get_tags()
                .and_then(|tags| {
                    tags.get::<gst::tags::Artist>()
                        .and_then(|value| value.get().map(ToString::to_string))
                })
                .or_else(|| media_artist.clone())
                .unwrap_or_else(get_default_chapter_title);
            write_fmt!(destination, "    PERFORMER \"{}\"\n", &artist);

            if let Some((start, _end)) = chapter.get_start_stop_times() {
                let start_ts = Timestamp4Humans::from_nano(start as u64);
                write_fmt!(
                    destination,
                    "    INDEX 01 {:02}:{:02}:{:02}\n",
                    start_ts.h * 60 + start_ts.m,
                    start_ts.s,
                    (f64::from((start_ts.ms * 1_000 + start_ts.us) * 1_000 + start_ts.nano)
                        / 1_000_000_000f64
                        * 75f64)
                        .round() // frame nb (75 frames/s for Cue Sheets)
                );
            }
        }

        Ok(())
    }
}
