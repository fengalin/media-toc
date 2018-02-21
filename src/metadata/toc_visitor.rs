extern crate gstreamer as gst;

#[derive(Debug)]
pub enum TocVisit {
    EnteringChildren,
    LeavingChildren,
    Node(gst::TocEntry),
}

impl PartialEq for TocVisit {
    fn eq(&self, other: &TocVisit) -> bool {
        match self {
            &TocVisit::EnteringChildren => match other {
                &TocVisit::EnteringChildren => true,
                _ => false,
            }
            &TocVisit::LeavingChildren => match other {
                &TocVisit::LeavingChildren => true,
                _ => false,
            }
            &TocVisit::Node(ref entry) => {
                match other {
                    &TocVisit::Node(ref other_entry) => {
                        (entry.get_uid() == other_entry.get_uid())
                    }
                    _ => false,
                }
            }
        }
    }
}

struct TocEntryIter {
    entries: Vec<gst::TocEntry>,
    index: usize,
}

impl TocEntryIter {
    fn from(entries: Vec<gst::TocEntry>) -> Self {
        Self {
            entries,
            index: 0,
        }
    }

    fn next(&mut self) -> Option<(gst::TocEntry, usize)> {
        if self.index >= self.entries.len() {
            return None;
        }

        let result = Some((self.entries[self.index].clone(), self.index));
        self.index += 1;
        result
    }
}

pub struct TocVisitor {
    stack: Vec<TocEntryIter>,
    next_to_push: Option<TocEntryIter>,
}

impl TocVisitor {
    pub fn new(toc: &gst::Toc) -> TocVisitor {
        let entries = toc.get_entries();
        let next_to_push = if !entries.is_empty() {
            Some(TocEntryIter::from(entries))
        } else {
            None
        };

        TocVisitor {
            stack: Vec::new(),
            next_to_push,
        }
    }

    // panics if expected structure not found
    pub fn enter_chapters(&mut self) {
        // Skip edition entry and enter chapters
        assert_eq!(Some(TocVisit::EnteringChildren), self.next());
        match self.next() {
            Some(TocVisit::Node(entry)) => {
                assert_eq!(gst::TocEntryType::Edition, entry.get_entry_type());
            }
            _ => panic!("TocVisitor::enter_chapters unexpected root toc entry"),
        }
        assert_eq!(Some(TocVisit::EnteringChildren), self.next());
    }

    pub fn next(&mut self) -> Option<TocVisit> {
        match self.next_to_push.take() {
            None => {
                if self.stack.is_empty() {
                    // Nothing left to be done
                    None
                } else {
                    let mut iter = self.stack.pop().unwrap();
                    match iter.next() {
                        Some((entry, _index)) => {
                            self.stack.push(iter);
                            let subentries = entry.get_sub_entries();
                            if !subentries.is_empty() {
                                self.next_to_push = Some(TocEntryIter::from(subentries));
                            }
                            Some(TocVisit::Node(entry))
                        }
                        None => Some(TocVisit::LeavingChildren),
                    }
                }
            }
            Some(next_to_push) => {
                self.stack.push(next_to_push);
                Some(TocVisit::EnteringChildren)
            }
        }
    }

    // Flattens the tree structure and get chapters in order
    pub fn next_chapter(&mut self) -> Option<gst::TocEntry> {
        loop {
            match self.next() {
                Some(toc_visit) => match toc_visit {
                    TocVisit::Node(entry) => match entry.get_entry_type() {
                        gst::TocEntryType::Chapter => return Some(entry),
                        _ => (),
                    },
                    _ => (),
                }
                None => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate gstreamer as gst;
    use gstreamer::{Toc, TocEntry, TocEntryType, TocScope};

    use super::*;

    #[test]
    fn subchapters() {
        gst::init().unwrap();

        let mut toc = Toc::new(TocScope::Global);
        {
            let mut edition = TocEntry::new(TocEntryType::Edition, "edition");

            let mut chapter_1 = TocEntry::new(TocEntryType::Chapter, "1");
            let chapter_1_1 = TocEntry::new(TocEntryType::Chapter, "1.1");
            let chapter_1_2 = TocEntry::new(TocEntryType::Chapter, "1.2");
            chapter_1.get_mut().unwrap().append_sub_entry(chapter_1_1);
            chapter_1.get_mut().unwrap().append_sub_entry(chapter_1_2);
            edition.get_mut().unwrap().append_sub_entry(chapter_1);

            let mut chapter_2 = TocEntry::new(TocEntryType::Chapter, "2");
            let chapter_2_1 = TocEntry::new(TocEntryType::Chapter, "2.1");
            let chapter_2_2 = TocEntry::new(TocEntryType::Chapter, "2.2");
            chapter_2.get_mut().unwrap().append_sub_entry(chapter_2_1);
            chapter_2.get_mut().unwrap().append_sub_entry(chapter_2_2);
            edition.get_mut().unwrap().append_sub_entry(chapter_2);

            toc.get_mut().unwrap().append_entry(edition);
        }

        let mut toc_visitor = TocVisitor::new(&toc);
        assert_eq!(Some(TocVisit::EnteringChildren), toc_visitor.next());
        assert_eq!(
            Some(TocVisit::Node(TocEntry::new(TocEntryType::Edition, "edition"))),
            toc_visitor.next(),
        );

        assert_eq!(Some(TocVisit::EnteringChildren), toc_visitor.next());
        assert_eq!(
            Some(TocVisit::Node(TocEntry::new(TocEntryType::Chapter, "1"))),
            toc_visitor.next(),
        );
        assert_eq!(Some(TocVisit::EnteringChildren), toc_visitor.next());
        assert_eq!(
            Some(TocVisit::Node(TocEntry::new(TocEntryType::Chapter, "1.1"))),
            toc_visitor.next(),
        );
        assert_eq!(
            Some(TocVisit::Node(TocEntry::new(TocEntryType::Chapter, "1.2"))),
            toc_visitor.next(),
        );
        assert_eq!(Some(TocVisit::LeavingChildren), toc_visitor.next());

        assert_eq!(
            Some(TocVisit::Node(TocEntry::new(TocEntryType::Chapter, "2"))),
            toc_visitor.next(),
        );
        assert_eq!(Some(TocVisit::EnteringChildren), toc_visitor.next());
        assert_eq!(
            Some(TocVisit::Node(TocEntry::new(TocEntryType::Chapter, "2.1"))),
            toc_visitor.next(),
        );
        assert_eq!(
            Some(TocVisit::Node(TocEntry::new(TocEntryType::Chapter, "2.2"))),
            toc_visitor.next(),
        );
        assert_eq!(Some(TocVisit::LeavingChildren), toc_visitor.next()); // sub chapters

        assert_eq!(Some(TocVisit::LeavingChildren), toc_visitor.next()); // chapters

        assert_eq!(Some(TocVisit::LeavingChildren), toc_visitor.next()); // edition
        assert!(toc_visitor.next().is_none());
    }
}
