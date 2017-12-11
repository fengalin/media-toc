extern crate glib;
use glib::Cast;

extern crate gstreamer as gst;
use gstreamer::{TagSetterExt, Toc, TocEntry, TocEntryType, TocScope, TocSetterExt};

use std::collections::HashMap;

use std::i64;

use super::{Chapter, Exporter};

static EXTENSION: &'static str = "toc.mkv";
static AUDIO_EXTENSION: &'static str = "toc.mka";

pub struct MatroskaTocFormat {
}

impl MatroskaTocFormat {
    pub fn get_extension() -> &'static str {
        EXTENSION
    }

    pub fn get_audio_extension() -> &'static str {
        AUDIO_EXTENSION
    }

    pub fn new() -> Self {
        MatroskaTocFormat{}
    }
}

impl Exporter for MatroskaTocFormat {
    fn export(&self,
        metadata: &HashMap<String, String>,
        chapters: &[Chapter],
        destination: &gst::Element,
    ) {
        {
            let tag_setter =
                destination.clone().dynamic_cast::<gst::TagSetter>()
                    .expect("MatroskaTocFormat::export muxer is not a TagSetter");

            let mut tag_list = gst::TagList::new();
            {
                let tag_list = tag_list.get_mut().unwrap();
                if let Some(title) = metadata.get(super::METADATA_TITLE) {
                    tag_list.add::<gst::tags::Title>(&title.as_str(), gst::TagMergeMode::Append);
                }
                if let Some(artist) = metadata.get(super::METADATA_ARTIST) {
                    tag_list.add::<gst::tags::Artist>(&artist.as_str(), gst::TagMergeMode::Append);
                }
            }

            tag_setter.merge_tags(&tag_list, gst::TagMergeMode::Append)
        }

        {
            let toc_setter =
                destination.clone().dynamic_cast::<gst::TocSetter>()
                    .expect("MatroskaTocFormat::export muxer is not a TocSetter");

            toc_setter.reset();

            let mut toc = Toc::new(TocScope::Global);
            let mut toc_entry = TocEntry::new(TocEntryType::Edition, "00");
            {
                let toc_entry = toc_entry.get_mut().unwrap();

                let mut min_pos = i64::MAX;
                let mut max_pos = 0i64;

                for (index, chapter) in chapters.iter().enumerate() {
                    let mut toc_sub_entry = TocEntry::new(
                        TocEntryType::Chapter,
                        &format!("{:02}", index + 1),
                    );

                    let mut tag_list = gst::TagList::new();
                    tag_list.get_mut().unwrap()
                        .add::<gst::tags::Title>(
                            &chapter.get_title().unwrap_or(super::DEFAULT_TITLE),
                            gst::TagMergeMode::Append,
                        );
                    toc_sub_entry.get_mut().unwrap()
                        .set_tags(tag_list);

                    let start = chapter.start.nano_total as i64;
                    let end = chapter.end.nano_total as i64;
                    toc_sub_entry.get_mut().unwrap().set_start_stop_times(start, end);

                    if start < min_pos {
                        min_pos = start;
                    }
                    if end > max_pos {
                        max_pos = end;
                    }

                    toc_entry.append_sub_entry(toc_sub_entry);
                }
            }

            toc.get_mut().unwrap().append_entry(toc_entry);

            toc_setter.set_toc(&toc);
        }
    }
}
