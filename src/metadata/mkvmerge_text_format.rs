use gettextrs::gettext;
use gstreamer as gst;

use nom;
use nom::types::CompleteStr;
use nom::{AtEof, InputLength};

use std::io::{Read, Write};

use super::{
    get_default_chapter_title, parse_timestamp, MediaInfo, Reader, Timestamp, TocVisitor, Writer,
};

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

fn new_chapter(nb: usize, start_ts: Timestamp, title: &str) -> gst::TocEntry {
    let mut chapter = gst::TocEntry::new(gst::TocEntryType::Chapter, &format!("{:02}", nb));
    let start = start_ts.nano_total as i64;
    chapter
        .get_mut()
        .unwrap()
        .set_start_stop_times(start, start);

    let mut tag_list = gst::TagList::new();
    tag_list
        .get_mut()
        .unwrap()
        .add::<gst::tags::Title>(&title, gst::TagMergeMode::Replace);
    chapter.get_mut().unwrap().set_tags(tag_list);
    chapter
}

named!(parse_chapter<CompleteStr, gst::TocEntry>,
    do_parse!(
        tag!(CHAPTER_TAG) >>
        nb1: flat_map!(take!(2), parse_to!(usize)) >>
        tag!("=") >>
        start: flat_map!(take_until_either!("\r\n"), parse_timestamp) >>
        eat_separator!("\r\n") >>
        tag!(CHAPTER_TAG) >>
        nb: verify!(flat_map!(take!(2), parse_to!(usize)), |nb2:usize| nb1 == nb2) >>
        tag!(NAME_TAG) >>
        tag!("=") >>
        title: take_until_either!("\r\n") >>
        opt!(eat_separator!("\r\n")) >>
        (new_chapter(nb, start, &title))
    )
);

#[test]
fn parse_chapter_test() {
    use nom::InputLength;
    gst::init().unwrap();

    let res = parse_chapter(CompleteStr("CHAPTER01=00:00:01.000\nCHAPTER01NAME=test\n"));
    let (i, toc_entry) = res.unwrap();
    assert_eq!(0, i.input_len());
    assert_eq!(1_000_000_000, toc_entry.get_start_stop_times().unwrap().0);
    assert_eq!(
        Some("test".to_owned()),
        toc_entry.get_tags().and_then(|tags| tags
            .get::<gst::tags::Title>()
            .map(|tag| tag.get().unwrap().to_owned())),
    );

    let res = parse_chapter(CompleteStr(
        "CHAPTER01=00:00:01.000\r\nCHAPTER01NAME=test\r\n",
    ));
    let (i, toc_entry) = res.unwrap();
    assert_eq!(0, i.input_len());
    assert_eq!(1_000_000_000, toc_entry.get_start_stop_times().unwrap().0);
    assert_eq!(
        Some("test".to_owned()),
        toc_entry.get_tags().and_then(|tags| tags
            .get::<gst::tags::Title>()
            .map(|tag| tag.get().unwrap().to_owned())),
    );

    let res = parse_chapter(CompleteStr("CHAPTER0x=00:00:01.000"));
    let err = res.unwrap_err();
    if let nom::Err::Error(nom::Context::Code(i, code)) = err {
        assert_eq!(CompleteStr("0x"), i);
        // FIXME: change `nom::ErrorKind::MapOpt` to `nom::ErrorKind::ParseTo`
        // when this PR is merged: https://github.com/Geal/nom/pull/747
        assert_eq!(nom::ErrorKind::MapOpt, code);
    } else {
        panic!("unexpected error type returned");
    }

    let res = parse_chapter(CompleteStr("CHAPTER01=00:00:01.000\nCHAPTER02NAME=test\n"));
    let err = res.unwrap_err();
    if let nom::Err::Error(nom::Context::Code(i, code)) = err {
        assert_eq!(CompleteStr("02NAME=test\n"), i);
        assert_eq!(nom::ErrorKind::Verify, code);
    } else {
        panic!("unexpected error type returned");
    }
}

impl Reader for MKVMergeTextFormat {
    fn read(&self, info: &MediaInfo, source: &mut Read) -> Result<Option<gst::Toc>, String> {
        let error_msg = gettext("unexpected error reading mkvmerge text file.");
        let mut content = String::new();
        source.read_to_string(&mut content).map_err(|_| {
            error!("{}", error_msg);
            error_msg.clone()
        })?;

        if !content.is_empty() {
            let mut toc_edition = gst::TocEntry::new(gst::TocEntryType::Edition, "");
            let mut last_chapter: Option<gst::TocEntry> = None;
            let mut input = CompleteStr(&content[..]);

            while input.input_len() > 0 {
                let cur_chapter = match parse_chapter(input) {
                    Ok((i, cur_chapter)) => {
                        if i.input_len() == input.input_len() {
                            // No progress
                            if !i.at_eof() {
                                let msg = gettext("unexpected sequence starting with: {}")
                                    .replacen("{}", &i[..i.len().min(10)], 1);
                                error!("{}", msg);
                                return Err(msg);
                            }
                            break;
                        }
                        input = i;
                        cur_chapter
                    }
                    Err(err) => {
                        let msg = if let nom::Err::Error(nom::Context::Code(i, code)) = err {
                            match code {
                                // FIXME: change `nom::ErrorKind::MapOpt` to
                                // `nom::ErrorKind::ParseTo` when this PR is merged:
                                // https://github.com/Geal/nom/pull/747
                                nom::ErrorKind::MapOpt => gettext("expecting a number, found: {}")
                                    .replacen("{}", &i[..i.len().min(2)], 1),
                                nom::ErrorKind::Verify => gettext(
                                    "chapter numbers don't match for: {}",
                                ).replacen("{}", &i[..i.len().min(2)], 1),
                                _ => gettext("unexpected sequence starting with: {}").replacen(
                                    "{}",
                                    &i[..i.len().min(10)],
                                    1,
                                ),
                            }
                        } else {
                            error!("unknown error {:?}", err);
                            error_msg
                        };
                        error!("{}", msg);
                        return Err(msg);
                    }
                };

                if let Some(mut prev_chapter) = last_chapter.take() {
                    // Update previous chapter's end
                    let prev_start = prev_chapter.get_start_stop_times().unwrap().0;
                    let cur_start = cur_chapter.get_start_stop_times().unwrap().0;
                    prev_chapter
                        .get_mut()
                        .unwrap()
                        .set_start_stop_times(prev_start, cur_start);
                    // Add previous chapter to the Edition entry
                    toc_edition
                        .get_mut()
                        .unwrap()
                        .append_sub_entry(prev_chapter);
                }

                // Queue current chapter (will be added when next chapter start is known
                // or with the media's duration when the parsing is done)
                last_chapter = Some(cur_chapter);
            }

            // Update last_chapter
            last_chapter.take().map_or_else(
                || {
                    error!("{}", gettext("couldn't update last start position"));
                    Err(error_msg)
                },
                |mut last_chapter| {
                    let last_start = last_chapter.get_start_stop_times().unwrap().0;
                    last_chapter
                        .get_mut()
                        .unwrap()
                        .set_start_stop_times(last_start, info.duration as i64);
                    toc_edition
                        .get_mut()
                        .unwrap()
                        .append_sub_entry(last_chapter);

                    let mut toc = gst::Toc::new(gst::TocScope::Global);
                    toc.get_mut().unwrap().append_entry(toc_edition);
                    Ok(Some(toc))
                },
            )
        } else {
            // file is empty
            Ok(None)
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
                    }).unwrap_or_else(get_default_chapter_title);
                write_fmt!(destination, "{}{}={}\n", prefix, NAME_TAG, &title);
            }
        }

        Ok(())
    }
}
