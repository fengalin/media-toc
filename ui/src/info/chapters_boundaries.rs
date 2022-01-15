use log::debug;

use std::{collections::BTreeMap, fmt, ops::Deref};

use renderers::Timestamp;

#[derive(Clone, Copy, Debug)]
pub struct ChapterTimestamps {
    pub start: Timestamp,
    pub end: Timestamp,
}

impl ChapterTimestamps {
    pub fn new(start: Timestamp, end: Timestamp) -> Self {
        ChapterTimestamps { start, end }
    }

    pub fn new_from_u64(start: u64, end: u64) -> Self {
        ChapterTimestamps {
            start: Timestamp::new(start),
            end: Timestamp::new(end),
        }
    }
}

impl fmt::Display for ChapterTimestamps {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "start {}, end {}",
            self.start.for_humans().to_string(),
            self.end.for_humans().to_string(),
        )
    }
}

#[derive(Clone, Debug)]
pub struct Chapter {
    pub title: String,
    pub ts: ChapterTimestamps,
    pub iter: gtk::TreeIter,
}

impl PartialEq for Chapter {
    fn eq(&self, other: &Chapter) -> bool {
        self.title == other.title && self.ts.start == other.ts.start
    }
}

#[derive(Debug, PartialEq)]
pub struct SuccessiveChapters {
    pub prev: Option<Chapter>,
    pub next: Option<Chapter>,
}

pub struct ChaptersBoundaries(BTreeMap<Timestamp, SuccessiveChapters>);

impl ChaptersBoundaries {
    pub fn new() -> Self {
        ChaptersBoundaries(BTreeMap::new())
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn add_chapter<Title>(&mut self, ts: ChapterTimestamps, title: Title, iter: &gtk::TreeIter)
    where
        Title: ToString,
    {
        let title = title.to_string();
        debug!("add_chapter {}, {}", ts, title);

        // the chapter to add can share at most one boundary with a previous chapter
        let (found_start, prev_next_chapter) =
            self.0
                .get_mut(&ts.start)
                .map_or((false, None), |boundary_at_start| {
                    let prev_next_chapter = boundary_at_start.next.take();
                    boundary_at_start.next = Some(Chapter {
                        title: title.clone(),
                        ts,
                        iter: iter.clone(),
                    });
                    (true, prev_next_chapter)
                });

        if found_start {
            self.0.insert(
                ts.end,
                SuccessiveChapters {
                    prev: Some(Chapter {
                        title,
                        ts,
                        iter: iter.clone(),
                    }),
                    next: prev_next_chapter,
                },
            );
        } else {
            // no chapter at start
            let (end_exists, prev_chapter) = match self.0.get_mut(&ts.end) {
                Some(chapters_at_end) => {
                    // a boundary already exists at end
                    let prev_chapter = chapters_at_end.prev.take();
                    chapters_at_end.prev = Some(Chapter {
                        title: title.clone(),
                        ts,
                        iter: iter.clone(),
                    });
                    (true, prev_chapter)
                }
                None => (false, None),
            };

            self.0.insert(
                ts.start,
                SuccessiveChapters {
                    prev: prev_chapter,
                    next: Some(Chapter {
                        title: title.clone(),
                        ts,
                        iter: iter.clone(),
                    }),
                },
            );

            if !end_exists {
                self.0.insert(
                    ts.end,
                    SuccessiveChapters {
                        prev: Some(Chapter {
                            title,
                            ts,
                            iter: iter.clone(),
                        }),
                        next: None,
                    },
                );
            }
        }
    }

    pub fn remove_chapter(&mut self, ts: ChapterTimestamps) {
        let prev_chapter = self.0.get_mut(&ts.start).unwrap().prev.take();
        self.0.remove(&ts.start);

        let boundary_at_end = self.0.get_mut(&ts.end).unwrap();
        if prev_chapter.is_none() && boundary_at_end.next.is_none() {
            self.0.remove(&ts.end);
        } else {
            boundary_at_end.prev = prev_chapter;
        }
    }

    pub fn rename_chapter<Title>(&mut self, ts: ChapterTimestamps, new_title: Title)
    where
        Title: ToString,
    {
        let new_title = new_title.to_string();
        debug!("rename_chapter {}, {}", ts, new_title);

        self.0
            .get_mut(&ts.start)
            .unwrap()
            .next
            .as_mut()
            .unwrap()
            .title = new_title.clone();
        self.0
            .get_mut(&ts.end)
            .unwrap()
            .prev
            .as_mut()
            .unwrap()
            .title = new_title;
    }

    pub fn move_boundary(&mut self, boundary: Timestamp, target: Timestamp) {
        let chapters = self.0.remove(&boundary).unwrap();
        if self.0.insert(target, chapters).is_some() {
            panic!(
                "ChaptersBoundaries::move_boundary attempt to replace entry at {}",
                target
            );
        }
    }
}

impl Deref for ChaptersBoundaries {
    type Target = BTreeMap<Timestamp, SuccessiveChapters>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;
    use gtk::{self, glib, TreeStoreExt};

    use renderers::Timestamp;

    fn new_chapter(store: &gtk::TreeStore, title: &str, ts: ChapterTimestamps) -> Chapter {
        Chapter {
            title: title.to_owned(),
            ts,
            iter: store.append(None),
        }
    }

    #[test]
    fn chapters_boundaries() {
        if gtk::init().is_err() {
            // GTK initialization failure on Travis-CI's linux host
            return;
        }
        // fake store
        let store = gtk::TreeStore::new(&[glib::Type::Bool]);

        let mut boundaries = ChaptersBoundaries::new();

        assert!(boundaries.is_empty());

        // Add incrementally

        let chapter_1 = new_chapter(&store, "1", ChapterTimestamps::new_from_u64(0, 1));
        boundaries.add_chapter(chapter_1.ts, &chapter_1.title, &chapter_1.iter);
        assert_eq!(2, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_1.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(1)),
        );

        let chapter_2 = new_chapter(&store, "2", ChapterTimestamps::new_from_u64(1, 2));
        boundaries.add_chapter(chapter_2.ts, &chapter_2.title, &chapter_2.iter);
        assert_eq!(3, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_1.clone()),
                next: Some(chapter_2.clone()),
            }),
            boundaries.get(&Timestamp::new(1)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_2.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(2)),
        );

        let chapter_3 = new_chapter(&store, "3", ChapterTimestamps::new_from_u64(2, 4));
        boundaries.add_chapter(chapter_3.ts, &chapter_3.title, &chapter_3.iter);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_1.clone()),
                next: Some(chapter_2.clone()),
            }),
            boundaries.get(&Timestamp::new(1)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_2.clone()),
                next: Some(chapter_3.clone()),
            }),
            boundaries.get(&Timestamp::new(2)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_3.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(4)),
        );

        // Rename
        let chapter_r2 = new_chapter(&store, "r2", ChapterTimestamps::new_from_u64(1, 2));
        boundaries.rename_chapter(chapter_r2.ts, &chapter_r2.title);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_1.clone()),
                next: Some(chapter_r2.clone()),
            }),
            boundaries.get(&Timestamp::new(1)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r2.clone()),
                next: Some(chapter_3.clone()),
            }),
            boundaries.get(&Timestamp::new(2)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_3.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(4)),
        );

        let chapter_r1 = new_chapter(&store, "r1", ChapterTimestamps::new_from_u64(0, 1));
        boundaries.rename_chapter(chapter_r1.ts, &chapter_r1.title);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_r1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r1.clone()),
                next: Some(chapter_r2.clone()),
            }),
            boundaries.get(&Timestamp::new(1)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r2.clone()),
                next: Some(chapter_3.clone()),
            }),
            boundaries.get(&Timestamp::new(2)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_3.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(4)),
        );

        let chapter_r3 = new_chapter(&store, "r3", ChapterTimestamps::new_from_u64(2, 4));
        boundaries.rename_chapter(chapter_r3.ts, &chapter_r3.title);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_r1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r1.clone()),
                next: Some(chapter_r2.clone()),
            }),
            boundaries.get(&Timestamp::new(1)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r2.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&Timestamp::new(2)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(4)),
        );

        // Remove in the middle
        boundaries.remove_chapter(ChapterTimestamps::new_from_u64(1, 2));
        assert_eq!(3, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_r1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r1.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&Timestamp::new(2)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(4)),
        );

        // Add in the middle
        let chapter_n2 = new_chapter(&store, "n2", ChapterTimestamps::new_from_u64(1, 2));
        boundaries.add_chapter(chapter_n2.ts, &chapter_n2.title, &chapter_n2.iter);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_r1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r1.clone()),
                next: Some(chapter_n2.clone()),
            }),
            boundaries.get(&Timestamp::new(1)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_n2.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&Timestamp::new(2)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(4)),
        );

        // Remove first
        boundaries.remove_chapter(ChapterTimestamps::new_from_u64(0, 1));
        assert_eq!(3, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_n2.clone()),
            }),
            boundaries.get(&Timestamp::new(1)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_n2.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&Timestamp::new(2)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(4)),
        );

        // Add first
        let chapter_n1 = new_chapter(&store, "n1", ChapterTimestamps::new_from_u64(0, 1));
        boundaries.add_chapter(chapter_n1.ts, &chapter_n1.title, &chapter_n1.iter);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_n1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_n1.clone()),
                next: Some(chapter_n2.clone()),
            }),
            boundaries.get(&Timestamp::new(1)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_n2.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&Timestamp::new(2)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(4)),
        );

        // Remove last
        boundaries.remove_chapter(ChapterTimestamps::new_from_u64(2, 4));
        assert_eq!(3, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: None,
                next: Some(chapter_n1.clone()),
            }),
            boundaries.get(&Timestamp::new(0)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_n1.clone()),
                next: Some(chapter_n2.clone()),
            }),
            boundaries.get(&Timestamp::new(1)),
        );
        assert_eq!(
            Some(&SuccessiveChapters {
                prev: Some(chapter_n2.clone()),
                next: None,
            }),
            boundaries.get(&Timestamp::new(4)),
        );
    }
}
