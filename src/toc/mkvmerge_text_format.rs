extern crate lazy_static;

use std::collections::HashMap;

use std::io::{Read, Write};

use super::{Chapter, Exporter, Importer, Timestamp};

static EXTENSION: &'static str = "txt";

static CHAPTER_TAG: &'static str = "CHAPTER";
const CHAPTER_NB_LEN: usize = 2;
static NAME_TAG: &'static str = "NAME";

lazy_static! {
    static ref CHAPTER_TAG_LEN: usize = CHAPTER_TAG.len();
}

pub struct MKVMergeTextFormat {
}

impl MKVMergeTextFormat {
    pub fn get_extension() -> &'static str {
        &EXTENSION
    }

    pub fn new_as_boxed() -> Box<Self> {
        Box::new(MKVMergeTextFormat{})
    }
}

// FIXME: handle errors

impl Importer for MKVMergeTextFormat {
    fn read(&self,
        duration: u64,
        source: &mut Read,
        _metadata: &mut HashMap<String, String>,
        chapters: &mut Vec<Chapter>,
    ) {
        let mut content = String::new();
        source.read_to_string(&mut content)
            .expect("MKVMergeTextFormat::read failed to read source content");

        chapters.clear();

        for line in content.lines() {
            let mut parts: Vec<&str> = line.trim().splitn(2, '=').collect();
            if parts.len() == 2 {
                let tag = parts[0];
                let value = parts[1];
                if tag.starts_with(CHAPTER_TAG) &&
                    tag.len() >= *CHAPTER_TAG_LEN + CHAPTER_NB_LEN
                {
                    let chapter_nb =
                        match tag[*CHAPTER_TAG_LEN..*CHAPTER_TAG_LEN + CHAPTER_NB_LEN]
                            .parse::<usize>()
                        {
                            Ok(chapter_nb) => chapter_nb,
                            Err(_) => panic!(
                                "MKVMergeTextFormat::read couldn't find chapter nb for: {}",
                                line,
                            ),
                        };

                    if tag.ends_with(NAME_TAG) {
                        if chapter_nb <= chapters.len() {
                            chapters[chapter_nb - 1].set_title(value);
                        } else {
                            panic!("MKVMergeTextFormat::read inconsistent chapter nb for: {}",
                                line,
                            );
                        }
                    } else {
                        // new chapter
                        if chapter_nb == chapters.len() + 1 {
                            let mut chapter = Chapter::empty();
                            let start = Timestamp::from_string(&value);

                            if chapter_nb > 1 {
                                // update previous chapter's end
                                chapters.get_mut(chapter_nb - 2)
                                    .expect("MKVMergeTextFormat::read inconsistent numbering")
                                    .end = start.clone();
                            }

                            chapter.start = start;
                            chapters.push(chapter);
                        } else {
                            panic!("MKVMergeTextFormat::read inconsistent chapter nb for: {}",
                                line,
                            );
                        }
                    }
                } else {
                    panic!("MKVMergeTextFormat::read unexpected format for: {}", line);
                }
            } else {
                panic!("MKVMergeTextFormat::read expected '=' for: {}", line);
            }
        }

        match chapters.last_mut() {
            Some(last_chapter) => last_chapter.end = Timestamp::from_nano(duration),
            None => (),
        };
    }
}

impl Exporter for MKVMergeTextFormat {
    fn extension(&self) -> &'static str {
        MKVMergeTextFormat::get_extension()
    }

    fn write(&self,
        _metadata: &HashMap<String, String>,
        chapters: &[Chapter],
        destination: &mut Write
    ) {
        for (index, chapter) in chapters.iter().enumerate() {
            let prefix = format!("{}{:02}", CHAPTER_TAG, index + 1);
            destination.write_fmt(
                format_args!("{}={}\n",
                    prefix,
                    chapter.start.format_with_hours(),
                ))
                .expect("MKVMergeTextFormat::write clicked, failed to write to file");
            if let Some(title) = chapter.get_title() {
                destination.write_fmt(format_args!("{}{}={}\n", prefix, NAME_TAG, title))
                    .expect("MKVMergeTextFormat::write clicked, failed to write to file");
            }
        }
    }
}
