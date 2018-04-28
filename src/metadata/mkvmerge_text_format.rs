use gettextrs::gettext;
use gstreamer as gst;

use nom;

use std::io::{Read, Write};

use super::{get_default_chapter_title, MediaInfo, Reader, Timestamp, TocVisitor, Writer};

static EXTENSION: &'static str = "txt";

static CHAPTER_TAG: &'static str = "CHAPTER";
static NAME_TAG: &'static str = "NAME";

pub struct MKVMergeTextFormat {}

impl MKVMergeTextFormat {
    pub fn get_extension() -> &'static str {
        EXTENSION
    }

    pub fn new_as_boxed() -> Box<Self> {
        Box::new(MKVMergeTextFormat {})
    }
}

fn new_chapter(nb: usize, start: Result<Timestamp, ()>, title: &str) -> gst::TocEntry {
    // FIXME: handle Timestamp conversion error in place
    let start = start.unwrap().nano_total;

    let mut chapter = gst::TocEntry::new(gst::TocEntryType::Chapter, &format!("{:02}", nb));
    chapter
        .get_mut()
        .unwrap()
        .set_start_stop_times(start as i64, start as i64);

    let mut tag_list = gst::TagList::new();
    tag_list
        .get_mut()
        .unwrap()
        .add::<gst::tags::Title>(&title, gst::TagMergeMode::Replace);
    chapter.get_mut().unwrap().set_tags(tag_list);
    chapter
}

named!(chapter<nom::types::CompleteStr, gst::TocEntry>,
    do_parse!(
        tag!(CHAPTER_TAG) >>
        nb1: flat_map!(nom::digit, parse_to!(usize)) >>
        tag!("=") >>
        start: map!(
            take_until_and_consume1!("\n"),
            |start_str| Timestamp::from_string(start_str.trim())
        ) >>
        tag!(CHAPTER_TAG) >>
        nb: verify!(flat_map!(nom::digit, parse_to!(usize)), |nb2:usize| nb1 == nb2) >>
        tag!(NAME_TAG) >>
        tag!("=") >>
        title: take_until_and_consume!("\n") >>
        (new_chapter(nb, start, &title))
    )
);

named!(chapters<nom::types::CompleteStr, (gst::TocEntry, Option<gst::TocEntry>) >,
    fold_many0!(
        chapter,
        (gst::TocEntry::new(gst::TocEntryType::Edition, ""), None),
        |mut acc: (gst::TocEntry, Option<gst::TocEntry>), cur_chapter: gst::TocEntry| {
            if let Some(mut prev_chapter) = acc.1.take() {
                // Update previous chapter's end
                let prev_start = prev_chapter.get_start_stop_times().unwrap().0;
                let cur_start = cur_chapter.get_start_stop_times().unwrap().0;
                prev_chapter
                    .get_mut()
                    .unwrap()
                    .set_start_stop_times(prev_start, cur_start);
                // Add previous chapter to the Edition entry
                acc.0.get_mut().unwrap().append_sub_entry(prev_chapter);
            }
            // Queue current chapter (will be added when next chapter start is known
            // or with the media's duration when the parsing is done)
            acc.1 = Some(cur_chapter);
            acc
        }
    )
);

#[cfg_attr(feature = "cargo-clippy", allow(match_wild_err_arm))]
impl Reader for MKVMergeTextFormat {
    fn read(&self, info: &MediaInfo, source: &mut Read) -> Result<gst::Toc, String> {
        let error_msg = gettext("Failed to read mkvmerge text file.");

        let mut content = String::new();
        source.read_to_string(&mut content).map_err(|_| {
            error!("{}", error_msg);
            error_msg.clone()
        })?;

        match chapters(nom::types::CompleteStr(&content[..])) {
            Ok((_, (mut toc_edition, Some(mut last_chapter)))) => {
                let last_start = last_chapter.get_start_stop_times().unwrap().0;
                last_chapter
                    .get_mut()
                    .unwrap()
                    .set_start_stop_times(last_start, info.duration as i64);
                toc_edition.get_mut().unwrap().append_sub_entry(last_chapter);

                let mut toc = gst::Toc::new(gst::TocScope::Global);
                toc.get_mut().unwrap().append_entry(toc_edition);
                Ok(toc)
            }
            Ok((_, (_, None))) => {
                // file is empty FIXME: return None instead of the toc
                unimplemented!("Empty mkvmerge text file");
            }
            Err(_) => {
                error!("{}", error_msg);
                Err(error_msg.clone())
            }
        }
    }
}

macro_rules! write_fmt(
    ($dest:ident, $fmt:expr, $( $item:expr ),*) => {
        $dest.write_fmt(format_args!($fmt, $( $item ),*)).map_err(|_| {
            let msg = gettext("Failed to write mkvmerge text file");
            error!("{}", msg);
            msg
        })?;
    };
);

impl Writer for MKVMergeTextFormat {
    fn write(&self, info: &MediaInfo, destination: &mut Write) -> Result<(), String> {
        if info.toc.is_none() {
            let msg = gettext("The table of contents is empty");
            error!("{}", msg);
            return Err(msg);
        }

        let mut index = 0;
        let mut toc_visitor = TocVisitor::new(info.toc.as_ref().unwrap());
        while let Some(chapter) = toc_visitor.next_chapter() {
            if let Some((start, _end)) = chapter.get_start_stop_times() {
                index += 1;
                let prefix = format!("{}{:02}", CHAPTER_TAG, index);
                write_fmt!(
                    destination,
                    "{}={}\n",
                    prefix,
                    Timestamp::from_nano(start as u64).format_with_hours()
                );

                let title = chapter
                    .get_tags()
                    .and_then(|tags| {
                        tags.get::<gst::tags::Title>()
                            .map(|tag| tag.get().unwrap().to_owned())
                    })
                    .unwrap_or_else(|| get_default_chapter_title());
                write_fmt!(destination, "{}{}={}\n", prefix, NAME_TAG, &title);
            }
        }

        Ok(())
    }
}
