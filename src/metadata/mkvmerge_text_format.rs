extern crate gstreamer as gst;
extern crate lazy_static;

use std::io::{Read, Write};

use super::{MediaInfo, Reader, Timestamp, Writer};

static EXTENSION: &'static str = "txt";

static CHAPTER_TAG: &'static str = "CHAPTER";
const CHAPTER_NB_LEN: usize = 2;
static NAME_TAG: &'static str = "NAME";

lazy_static! {
    static ref CHAPTER_TAG_LEN: usize = CHAPTER_TAG.len();
}

pub struct MKVMergeTextFormat {}

impl MKVMergeTextFormat {
    pub fn get_extension() -> &'static str {
        EXTENSION
    }

    pub fn new_as_boxed() -> Box<Self> {
        Box::new(MKVMergeTextFormat {})
    }
}

// FIXME: handle errors
#[cfg_attr(feature = "cargo-clippy", allow(match_wild_err_arm))]
impl Reader for MKVMergeTextFormat {
    fn read(
        &self,
        info: &MediaInfo,
        source: &mut Read,
    ) -> Option<gst::Toc> {
        fn add_chapter(
            parent: &mut gst::TocEntry,
            mut nb: Option<usize>,
            start: u64,
            end: u64,
            mut title: Option<&str>,
        ) {
            let nb = nb
                .take()
                .expect("MKVMergeTextFormat::add_chapter no number for chapter");
            let title = title
                .take()
                .expect("MKVMergeTextFormat::add_chapter no title for chapter");

            let mut chapter = gst::TocEntry::new(gst::TocEntryType::Chapter, &format!("{:02}", nb));
            chapter
                .get_mut()
                .unwrap()
                .set_start_stop_times(start as i64, end as i64);

            let mut tag_list = gst::TagList::new();
            tag_list.get_mut().unwrap().add::<gst::tags::Title>(&title, gst::TagMergeMode::Replace);
            chapter.get_mut().unwrap().set_tags(tag_list);

            parent
                .get_mut()
                .unwrap()
                .append_sub_entry(chapter);
        }

        let mut content = String::new();
        source
            .read_to_string(&mut content)
            .expect("MKVMergeTextFormat::read failed reading source content");

        let mut toc_edition = gst::TocEntry::new(gst::TocEntryType::Edition, "");
        let mut last_nb = None;
        let mut last_start = None;
        let mut last_title = None;

        for line in content.lines() {
            let mut parts: Vec<&str> = line.trim().splitn(2, '=').collect();
            if parts.len() == 2 {
                let tag = parts[0];
                let value = parts[1];
                if tag.starts_with(CHAPTER_TAG) && tag.len() >= *CHAPTER_TAG_LEN + CHAPTER_NB_LEN {
                    let cur_nb = match tag[*CHAPTER_TAG_LEN..*CHAPTER_TAG_LEN+CHAPTER_NB_LEN]
                        .parse::<usize>()
                    {
                        Ok(chapter_nb) => chapter_nb,
                        Err(_) => panic!(
                            "MKVMergeTextFormat::read couldn't find chapter nb for: {}",
                            line,
                        ),
                    };

                    if tag.ends_with(NAME_TAG) {
                        last_title = Some(value);
                    } else {
                        // New chapter start
                        // First add previous if any, now that we know its end
                        let cur_start = Timestamp::from_string(value).nano_total;

                        if let Some(last_start) = last_start.take() {
                            // update previous chapter's end
                            add_chapter(
                                &mut toc_edition,
                                last_nb,
                                last_start,
                                cur_start,
                                last_title,
                            );
                        }

                        last_start = Some(cur_start);
                        last_nb = Some(cur_nb);
                    }
                } else {
                    panic!("MKVMergeTextFormat::read unexpected format for: {}", line);
                }
            } else {
                panic!("MKVMergeTextFormat::read expected '=' for: {}", line);
            }
        }

        last_start.take().map(|last_start| {
            add_chapter(
                &mut toc_edition,
                last_nb,
                last_start,
                info.duration,
                last_title,
            );
            let mut toc = gst::Toc::new(gst::TocScope::Global);
            toc.get_mut().unwrap().append_entry(toc_edition);
            toc
        })
    }
}

impl Writer for MKVMergeTextFormat {
    fn write(&self, info: &MediaInfo, destination: &mut Write) {
        if let Some(ref toc) = info.toc {
            let top_entries = toc.get_entries();
            assert_eq!(top_entries.len(), 1);
            let edition = &top_entries[0];
            assert_eq!(edition.get_entry_type(), gst::TocEntryType::Edition);
            for (index, chapter) in edition.get_sub_entries().iter().enumerate() {
                assert_eq!(chapter.get_entry_type(), gst::TocEntryType::Chapter);
                if let Some((start, _end)) = chapter.get_start_stop_times() {
                    let prefix = format!("{}{:02}", CHAPTER_TAG, index + 1);
                    destination
                        .write_fmt(format_args!(
                            "{}={}\n",
                            prefix,
                            Timestamp::from_nano(start as u64).format_with_hours(),
                        ))
                        .expect("MKVMergeTextFormat::write clicked, failed to write to file");

                    let title = chapter.get_tags().map(|tags| {
                        tags.get::<gst::tags::Title>().map(|tag| {
                            tag.get().unwrap().to_owned()
                        }).unwrap_or(super::DEFAULT_TITLE.to_owned())
                    }).unwrap_or(super::DEFAULT_TITLE.to_owned());
                    destination
                        .write_fmt(format_args!("{}{}={}\n", prefix, NAME_TAG, &title))
                        .expect("MKVMergeTextFormat::write clicked, failed to write to file");
                }
            }
        }
    }
}
