use gettextrs::gettext;
use gstreamer as gst;

use std::io::{Read, Write};

use super::{MediaInfo, Reader, Timestamp, TocVisitor, Writer};

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

#[cfg_attr(feature = "cargo-clippy", allow(match_wild_err_arm))]
impl Reader for MKVMergeTextFormat {
    fn read(
        &self,
        info: &MediaInfo,
        source: &mut Read,
    ) -> Result<gst::Toc, String> {
        fn add_chapter(
            parent: &mut gst::TocEntry,
            mut nb: Option<usize>,
            start: u64,
            end: u64,
            mut title: Option<&str>,
        ) {
            let nb = nb.take().unwrap();
            let title = title.take().unwrap();

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

        let error_msg = gettext("Failed to read mkvmerge text file.");

        let mut content = String::new();
        if source.read_to_string(&mut content).is_err() {
            error!("{}", error_msg);
            return Err(error_msg);
        }

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
                        Err(_) => {
                            error!("{}", gettext("couldn't find chapter nb for: {}")
                                .replacen("{}", line, 1)
                            );
                            return Err(error_msg);
                        }
                    };

                    if tag.ends_with(NAME_TAG) {
                        last_title = Some(value);
                    } else {
                        // New chapter start
                        // First add previous if any, now that we know its end
                        let cur_start = match Timestamp::from_string(value) {
                            Ok(timestamp) => timestamp.nano_total,
                            Err(()) => {
                                error!("{}", gettext("unexpected timestamp: {}")
                                    .replacen("{}", value, 1)
                                );
                                return Err(error_msg);
                            }
                        };

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
                    error!("{}", gettext("unexpected format for: {}").replacen("{}", line, 1));
                    return Err(error_msg);
                }
            } else {
                error!("{}", gettext("expected '=' in: {}").replacen("{}", line, 1));
                return Err(error_msg);
            }
        }

        last_start.take().map_or_else(
            || {
                error!("{}", gettext("couldn't update last start position"));
                Err(error_msg)
            },
            |last_start| {
                add_chapter(
                    &mut toc_edition,
                    last_nb,
                    last_start,
                    info.duration,
                    last_title,
                );
                let mut toc = gst::Toc::new(gst::TocScope::Global);
                toc.get_mut().unwrap().append_entry(toc_edition);
                Ok(toc)
            }
        )
    }
}

macro_rules! write_fmt(
    ($dest:ident, $fmt:expr, $( $item:expr ),*) => {
        if $dest.write_fmt(format_args!($fmt, $( $item ),*)).is_err() {
            return Err(gettext("Failed to write mkvmerge text file"));
        }
    };
);

impl Writer for MKVMergeTextFormat {
    fn write(&self, info: &MediaInfo, destination: &mut Write) -> Result<(), String> {
        if info.toc.is_none() {
            return Err(gettext("The table of contents is empty"));
        }

        let mut index = 0;
        let mut toc_visitor = TocVisitor::new(info.toc.as_ref().unwrap());
        while let Some(chapter) = toc_visitor.next_chapter() {
            if let Some((start, _end)) = chapter.get_start_stop_times() {
                index += 1;
                let prefix = format!("{}{:02}", CHAPTER_TAG, index);
                write_fmt!(destination, "{}={}\n",
                    prefix,
                    Timestamp::from_nano(start as u64).format_with_hours()
                );

                let title = chapter.get_tags().map_or(None, |tags| {
                    tags.get::<gst::tags::Title>().map(|tag| {
                        tag.get().unwrap().to_owned()
                    })
                }).unwrap_or(super::DEFAULT_TITLE.to_owned());
                write_fmt!(destination, "{}{}={}\n", prefix, NAME_TAG ,&title);
            }
        }

        Ok(())
    }
}
