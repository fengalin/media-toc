use gtk;

use std::collections::BTreeMap;

use std::ops::Deref;

#[derive(Clone, Debug)]
pub struct Chapter {
    pub title: String,
    pub iter: gtk::TreeIter,
}

impl PartialEq for Chapter {
    fn eq(&self, other: &Chapter) -> bool {
        self.title == other.title
    }
}

#[derive(Debug, PartialEq)]
pub struct SuccessiveChapters {
    pub prev: Option<Chapter>,
    pub next: Option<Chapter>,
}

pub struct ChaptersBoundaries(BTreeMap<u64, SuccessiveChapters>);

impl ChaptersBoundaries {
    pub fn new() -> Self {
        ChaptersBoundaries(BTreeMap::new())
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn add_chapter(&mut self, start: u64, end: u64, title: &str, iter: &gtk::TreeIter) {
        debug!("add_chapter {}, {}, {}", start, end, title);

        // the chapter to add can share at most one boundary with a previous chapter
        let (start_exists, next_chapter) = match self.0.get_mut(&start) {
            Some(chapters_at_start) => {
                // a boundary already exists at start
                let next_chapter = chapters_at_start.next.take();
                chapters_at_start.next = Some(Chapter {
                    title: title.to_owned(),
                    iter: iter.clone(),
                });
                (true, next_chapter)
            }
            None => (false, None),
        };

        if start_exists {
            self.0.insert(
                end,
                SuccessiveChapters {
                    prev: Some(Chapter {
                        title: title.to_owned(),
                        iter: iter.clone(),
                    }),
                    next: next_chapter,
                },
            );
        } else {
            // no chapter at start
            let (end_exists, prev_chapter) = match self.0.get_mut(&end) {
                Some(chapters_at_end) => {
                    // a boundary already exists at end
                    let prev_chapter = chapters_at_end.prev.take();
                    chapters_at_end.prev = Some(Chapter {
                        title: title.to_owned(),
                        iter: iter.clone(),
                    });
                    (true, prev_chapter)
                }
                None => (false, None),
            };

            self.0.insert(
                start,
                SuccessiveChapters {
                    prev: prev_chapter,
                    next: Some(Chapter {
                        title: title.to_owned(),
                        iter: iter.clone(),
                    }),
                },
            );

            if !end_exists {
                self.0.insert(
                    end,
                    SuccessiveChapters {
                        prev: Some(Chapter {
                            title: title.to_owned(),
                            iter: iter.clone(),
                        }),
                        next: None,
                    },
                );
            }
        }
    }

    pub fn remove_chapter(&mut self, start: u64, end: u64) {
        debug!("remove_chapter {}, {}", start, end);

        let prev_chapter = self.0.get_mut(&start).unwrap().prev.take();
        self.0.remove(&start);

        let can_remove_end = {
            let chapters_at_end = self.0.get_mut(&end).unwrap();
            if prev_chapter.is_none() && chapters_at_end.next.is_none() {
                true
            } else {
                chapters_at_end.prev = prev_chapter;
                false
            }
        };
        if can_remove_end {
            self.0.remove(&end);
        }
    }

    pub fn rename_chapter(&mut self, start: u64, end: u64, new_title: &str) {
        debug!("rename_chapter {}, {}, {}", start, end, new_title);

        self.0.get_mut(&start).unwrap().next.as_mut().unwrap().title = new_title.to_owned();
        self.0.get_mut(&end).unwrap().prev.as_mut().unwrap().title = new_title.to_owned();
    }

    pub fn move_boundary(&mut self, boundary: u64, to_position: u64) {
        let chapters = self.0.remove(&boundary).unwrap();
        if self.0.insert(to_position, chapters).is_some() {
            panic!(
                "ChaptersBoundaries::move_boundary attempt to replace entry at {}",
                to_position
            );
        }
    }
}

impl Deref for ChaptersBoundaries {
    type Target = BTreeMap<u64, SuccessiveChapters>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/* Discard these tests for now as it fails for Travis-CI's linux host
#[cfg(test)]
mod tests {
    use glib;
    use gtk;
    use gtk::TreeStoreExt;
    use super::*;

    fn new_chapter(store: &gtk::TreeStore, title: &str) -> Chapter {
        Chapter {
            title: title.to_owned(),
            iter: store.append(None),
        }
    }

    #[test]
    fn chapters_boundaries() {
        if gtk::init().is_err() {
            panic!("tests::chapters_boundaries failed to initialize GTK");
        }
        // fake store
        let store = gtk::TreeStore::new(&[glib::Type::Bool]);

        let mut boundaries = ChaptersBoundaries::new();

        assert!(boundaries.is_empty());

        // Add incrementally

        let chapter_1 = new_chapter(&store, "1");
        boundaries.add_chapter(0, 1, &chapter_1.title, &chapter_1.iter);
        assert_eq!(2, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_1.clone()),
                next: None,
            }),
            boundaries.get(&1),
        );

        let chapter_2 = new_chapter(&store, "2");
        boundaries.add_chapter(1, 2, &chapter_2.title, &chapter_2.iter);
        assert_eq!(3, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_1.clone()),
                next: Some(chapter_2.clone()),
            }),
            boundaries.get(&1),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_2.clone()),
                next: None,
            }),
            boundaries.get(&2),
        );

        let chapter_3 = new_chapter(&store, "3");
        boundaries.add_chapter(2, 4, &chapter_3.title, &chapter_3.iter);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_1.clone()),
                next: Some(chapter_2.clone()),
            }),
            boundaries.get(&1),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_2.clone()),
                next: Some(chapter_3.clone()),
            }),
            boundaries.get(&2),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_3.clone()),
                next: None,
            }),
            boundaries.get(&4),
        );

        // Rename
        let chapter_r2 = new_chapter(&store, "r2");
        boundaries.rename_chapter(1, 2, &chapter_r2.title);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_1.clone()),
                next: Some(chapter_r2.clone()),
            }),
            boundaries.get(&1),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r2.clone()),
                next: Some(chapter_3.clone()),
            }),
            boundaries.get(&2),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_3.clone()),
                next: None,
            }),
            boundaries.get(&4),
        );

        let chapter_r1 = new_chapter(&store, "r1");
        boundaries.rename_chapter(0, 1, &chapter_r1.title);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_r1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r1.clone()),
                next: Some(chapter_r2.clone()),
            }),
            boundaries.get(&1),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r2.clone()),
                next: Some(chapter_3.clone()),
            }),
            boundaries.get(&2),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_3.clone()),
                next: None,
            }),
            boundaries.get(&4),
        );

        let chapter_r3 = new_chapter(&store, "r3");
        boundaries.rename_chapter(2, 4, &chapter_r3.title);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_r1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r1.clone()),
                next: Some(chapter_r2.clone()),
            }),
            boundaries.get(&1),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r2.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&2),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&4),
        );

        // Remove in the middle
        boundaries.remove_chapter(1, 2);
        assert_eq!(3, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_r1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r1.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&2),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&4),
        );

        // Add in the middle
        let chapter_n2 = new_chapter(&store, "n2");
        boundaries.add_chapter(1, 2, &chapter_n2.title, &chapter_n2.iter);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_r1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r1.clone()),
                next: Some(chapter_n2.clone()),
            }),
            boundaries.get(&1),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_n2.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&2),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&4),
        );

        // Remove first
        boundaries.remove_chapter(0, 1);
        assert_eq!(3, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_n2.clone()),
            }),
            boundaries.get(&1),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_n2.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&2),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&4),
        );

        // Add first
        let chapter_n1 = new_chapter(&store, "n1");
        boundaries.add_chapter(0, 1, &chapter_n1.title, &chapter_n1.iter);
        assert_eq!(4, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_n1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_n1.clone()),
                next: Some(chapter_n2.clone()),
            }),
            boundaries.get(&1),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_n2.clone()),
                next: Some(chapter_r3.clone()),
            }),
            boundaries.get(&2),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_r3.clone()),
                next: None,
            }),
            boundaries.get(&4),
        );

        // Remove last
        boundaries.remove_chapter(2, 4);
        assert_eq!(3, boundaries.len());
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: None,
                next: Some(chapter_n1.clone()),
            }),
            boundaries.get(&0),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_n1.clone()),
                next: Some(chapter_n2.clone()),
            }),
            boundaries.get(&1),
        );
        assert_eq!(
            Some(&SuccessiveChapters{
                prev: Some(chapter_n2.clone()),
                next: None,
            }),
            boundaries.get(&4),
        );
    }
}
*/
