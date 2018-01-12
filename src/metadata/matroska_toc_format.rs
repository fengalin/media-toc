extern crate glib;
use glib::Cast;

extern crate gstreamer as gst;
use gstreamer::{TagSetterExt, Toc, TocEntry, TocEntryType, TocScope, TocSetterExt};

use std::i64;

use super::{Chapter, Exporter, MediaInfo};

static EXTENSION: &'static str = "toc.mkv";
static AUDIO_EXTENSION: &'static str = "toc.mka";

pub struct MatroskaTocFormat {}

impl MatroskaTocFormat {
    pub fn get_extension() -> &'static str {
        EXTENSION
    }

    pub fn get_audio_extension() -> &'static str {
        AUDIO_EXTENSION
    }

    pub fn new() -> Self {
        MatroskaTocFormat {}
    }
}

impl Exporter for MatroskaTocFormat {
    fn export(&self, info: &MediaInfo, chapters: &[Chapter], destination: &gst::Element) {
        {
            let tag_setter = destination
                .clone()
                .dynamic_cast::<gst::TagSetter>()
                .expect("MatroskaTocFormat::export muxer is not a TagSetter");

            tag_setter.merge_tags(&info.tags, gst::TagMergeMode::Replace)
        }

        {
            let toc_setter = destination
                .clone()
                .dynamic_cast::<gst::TocSetter>()
                .expect("MatroskaTocFormat::export muxer is not a TocSetter");

            let mut toc = Toc::new(TocScope::Global);
            let mut toc_entry = TocEntry::new(TocEntryType::Edition, "");
            {
                let toc_entry = toc_entry.get_mut().unwrap();

                for (index, chapter) in chapters.iter().enumerate() {
                    let mut toc_sub_entry =
                        TocEntry::new(TocEntryType::Chapter, &format!("{:02}", index + 1));

                    let mut tag_list = gst::TagList::new();
                    tag_list.get_mut().unwrap().add::<gst::tags::Title>(
                        &chapter.get_title().unwrap_or(super::DEFAULT_TITLE),
                        gst::TagMergeMode::Append,
                    );
                    toc_sub_entry.get_mut().unwrap().set_tags(tag_list);

                    let start = chapter.start.nano_total as i64;
                    let end = chapter.end.nano_total as i64;
                    toc_sub_entry
                        .get_mut()
                        .unwrap()
                        .set_start_stop_times(start, end);

                    toc_entry.append_sub_entry(toc_sub_entry);
                }
            }

            toc.get_mut().unwrap().append_entry(toc_entry);

            toc_setter.set_toc(&toc);
        }
    }
}
