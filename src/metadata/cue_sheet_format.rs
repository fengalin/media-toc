extern crate gstreamer as gst;

use std::io::Write;

use super::{MediaInfo, Timestamp, Writer};

static EXTENSION: &'static str = "cue";

pub struct CueSheetFormat {}

impl CueSheetFormat {
    pub fn get_extension() -> &'static str {
        EXTENSION
    }

    pub fn new_as_boxed() -> Box<Self> {
        Box::new(CueSheetFormat {})
    }
}

impl Writer for CueSheetFormat {
    fn write(&self, info: &MediaInfo, destination: &mut Write) {
        let media_title = info.get_title().map(|title| title.to_owned());
        if let Some(ref title) = media_title {
            destination
                .write_fmt(format_args!("TITLE \"{}\"\n", title))
                .expect("CueSheetFormat::write clicked, failed to write to file");
        }

        let media_artist = info.get_artist().map(|artist| artist.to_owned());
        if let Some(ref artist) = media_artist {
            destination
                .write_fmt(format_args!("PERFORMER \"{}\"\n", artist))
                .expect("CueSheetFormat::write clicked, failed to write to file");
        }

        let audio_codec = match info.get_audio_codec() {
            Some(audio_codec) => {
                if audio_codec.to_lowercase().find("mp3").is_some() {
                    "MP3"
                } else if audio_codec.to_lowercase().find("aiff").is_some() {
                    "AIFF"
                } else {
                    "WAVE"
                }
            }
            None => "WAVE",
        };
        destination
            .write_fmt(format_args!(
                "FILE \"{}\" {}\n",
                info.get_file_name(),
                audio_codec
            ))
            .expect("CueSheetFormat::write clicked, failed to write to file");

        if let Some(ref toc) = info.toc {
            let top_entries = toc.get_entries();
            assert_eq!(top_entries.len(), 1);
            let edition = &top_entries[0];
            assert_eq!(edition.get_entry_type(), gst::TocEntryType::Edition);
            for (index, chapter) in edition.get_sub_entries().iter().enumerate() {
                assert_eq!(chapter.get_entry_type(), gst::TocEntryType::Chapter);
                // FIXME: are there other TRACK types than AUDIO?
                destination
                    .write_fmt(format_args!("  TRACK{:02} AUDIO\n", index + 1))
                    .expect("CueSheetFormat::write clicked, failed to write to file");

                let title = chapter.get_tags().map_or(None, |tags| {
                    tags.get::<gst::tags::Title>().map(|tag| {
                        tag.get().unwrap().to_owned()
                    })
                })
                    .map_or(media_title.clone(), |track_title| Some(track_title))
                    .unwrap_or(super::DEFAULT_TITLE.to_owned());
                destination
                    .write_fmt(format_args!("    TITLE \"{}\"\n", &title))
                    .expect("CueSheetFormat::write clicked, failed to write to file");

                let artist = chapter.get_tags().map_or(None, |tags| {
                    tags.get::<gst::tags::Artist>().map(|tag| {
                        tag.get().unwrap().to_owned()
                    })
                })
                    .map_or(media_artist.clone(), |track_artist| Some(track_artist))
                    .unwrap_or(super::DEFAULT_TITLE.to_owned());
                destination
                    .write_fmt(format_args!("    PERFORMER \"{}\"\n", &artist))
                    .expect("CueSheetFormat::write clicked, failed to write to file");

                if let Some((start, _end)) = chapter.get_start_stop_times() {
                    let start_ts = Timestamp::from_nano(start as u64);
                    destination
                        .write_fmt(format_args!(
                            "    INDEX 01 {:02}:{:02}:{:02}\n",
                            start_ts.h * 60 + start_ts.m,
                            start_ts.s,
                            (((start_ts.ms * 1_000 + start_ts.us) * 1_000 + start_ts.nano) as f64
                                / 1_000_000_000f64 * 75f64)
                                .round() // frame nb (75 frames/s for Cue Sheets)
                        ))
                        .expect("CueSheetFormat::write clicked, failed to write to file");
                }
            }
        }
    }
}
