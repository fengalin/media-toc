use gettextrs::gettext;
use gstreamer as gst;

use gtk;
use gtk::prelude::*;

use std::{
    cell::RefCell,
    rc::Rc,
};

use crate::metadata::{get_default_chapter_title, Timestamp, TocVisitor};

use super::ChaptersBoundaries;

const START_COL: u32 = 0;
const END_COL: u32 = 1;
const TITLE_COL: u32 = 2;
const START_STR_COL: u32 = 3;
const END_STR_COL: u32 = 4;

pub struct ChapterEntry<'a> {
    store: &'a gtk::TreeStore,
    iter: &'a gtk::TreeIter,
}

impl<'a> ChapterEntry<'a> {
    pub fn new(store: &'a gtk::TreeStore, iter: &'a gtk::TreeIter) -> ChapterEntry<'a> {
        ChapterEntry { store, iter }
    }

    pub fn title(&self) -> String {
        ChapterEntry::get_title(self.store, self.iter)
    }

    pub fn start(&self) -> u64 {
        ChapterEntry::get_start(self.store, self.iter)
    }

    pub fn start_str(&self) -> String {
        ChapterEntry::get_start_str(self.store, self.iter)
    }

    pub fn start_ts(&self) -> Timestamp {
        Timestamp::from_nano(ChapterEntry::get_start(self.store, self.iter))
    }

    pub fn end(&self) -> u64 {
        ChapterEntry::get_end(self.store, self.iter)
    }

    pub fn end_str(&self) -> String {
        ChapterEntry::get_end_str(self.store, self.iter)
    }

    #[allow(dead_code)]
    pub fn end_ts(&self) -> Timestamp {
        Timestamp::from_nano(ChapterEntry::get_end(self.store, self.iter))
    }

    pub fn as_toc_entry(&self) -> gst::TocEntry {
        let mut toc_entry = gst::TocEntry::new(
            gst::TocEntryType::Chapter,
            &format!("{}", self.start_ts().nano_total),
        );
        toc_entry
            .get_mut()
            .unwrap()
            .set_start_stop_times(self.start() as i64, self.end() as i64);

        let mut tag_list = gst::TagList::new();
        tag_list
            .get_mut()
            .unwrap()
            .add::<gst::tags::Title>(&self.title().as_str(), gst::TagMergeMode::Replace);
        toc_entry.get_mut().unwrap().set_tags(tag_list);

        toc_entry
    }

    pub fn get_title(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> String {
        store
            .get_value(iter, TITLE_COL as i32)
            .get::<String>()
            .unwrap()
    }

    pub fn get_start(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> u64 {
        store
            .get_value(iter, START_COL as i32)
            .get::<u64>()
            .unwrap()
    }

    pub fn get_start_str(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> String {
        store
            .get_value(iter, START_STR_COL as i32)
            .get::<String>()
            .unwrap()
    }

    pub fn get_end(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> u64 {
        store.get_value(iter, END_COL as i32).get::<u64>().unwrap()
    }

    pub fn get_end_str(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> String {
        store
            .get_value(iter, END_STR_COL as i32)
            .get::<String>()
            .unwrap()
    }
}

pub struct ChapterTreeManager {
    store: gtk::TreeStore,
    iter: Option<gtk::TreeIter>,
    selected_iter: Option<gtk::TreeIter>,
    pub title_renderer: Option<gtk::CellRendererText>,
    boundaries: Rc<RefCell<ChaptersBoundaries>>,
}

impl ChapterTreeManager {
    pub fn new(store: gtk::TreeStore, boundaries: Rc<RefCell<ChaptersBoundaries>>) -> Self {
        ChapterTreeManager {
            store,
            iter: None,
            selected_iter: None,
            title_renderer: None,
            boundaries,
        }
    }

    pub fn init_treeview(&mut self, treeview: &gtk::TreeView) {
        treeview.set_model(Some(&self.store));
        self.title_renderer =
            Some(self.add_column(treeview, &gettext("Title"), TITLE_COL, true, true));
        self.add_column(treeview, &gettext("Start"), START_STR_COL, false, false);
        self.add_column(treeview, &gettext("End"), END_STR_COL, false, false);
    }

    fn add_column(
        &self,
        treeview: &gtk::TreeView,
        title: &str,
        col_id: u32,
        can_expand: bool,
        is_editable: bool,
    ) -> gtk::CellRendererText {
        let col = gtk::TreeViewColumn::new();
        col.set_title(title);

        let renderer = gtk::CellRendererText::new();
        if is_editable {
            renderer.set_property_editable(true);
            let store_clone = self.store.clone();
            renderer.connect_edited(move |_, tree_path, value| {
                if let Some(iter) = store_clone.get_iter(&tree_path) {
                    store_clone.set_value(&iter, TITLE_COL, &gtk::Value::from(value));
                }
            });
        }

        col.pack_start(&renderer, true);
        col.add_attribute(&renderer, "text", col_id as i32);
        if can_expand {
            col.set_min_width(70);
            col.set_expand(can_expand);
        } else {
            // align right
            renderer.set_property_xalign(1f32);
        }
        treeview.append_column(&col);

        renderer
    }

    pub fn get_selected_iter(&self) -> Option<gtk::TreeIter> {
        self.selected_iter.clone()
    }

    pub fn get_chapter_at_iter<'a: 'b, 'b>(&'a self, iter: &'a gtk::TreeIter) -> ChapterEntry<'b> {
        ChapterEntry::new(&self.store, iter)
    }

    pub fn get_iter(&self, tree_path: &gtk::TreePath) -> Option<gtk::TreeIter> {
        self.store.get_iter(tree_path)
    }

    pub fn unselect(&mut self) {
        self.selected_iter = None;
    }

    pub fn rewind(&mut self) {
        self.iter = self.store.get_iter_first();
    }

    pub fn clear(&mut self) {
        self.selected_iter = None;
        self.iter = None;
        self.boundaries.borrow_mut().clear();
        self.store.clear();
    }

    pub fn rename_selected_chapter(&mut self, new_title: &str) {
        if let Some(iter) = self.get_selected_iter() {
            self.boundaries.borrow_mut().rename_chapter(
                ChapterEntry::get_start(&self.store, &iter),
                ChapterEntry::get_end(&self.store, &iter),
                new_title,
            );
        }
    }

    pub fn replace_with(&mut self, toc: &Option<gst::Toc>) {
        self.clear();

        if let Some(ref toc) = *toc {
            let mut toc_visitor = TocVisitor::new(toc);
            if !toc_visitor.enter_chapters() {
                return;
            }

            // FIXME: handle hierarchical Tocs
            while let Some(chapter) = toc_visitor.next_chapter() {
                assert_eq!(gst::TocEntryType::Chapter, chapter.get_entry_type());

                if let Some((start, end)) = chapter.get_start_stop_times() {
                    let start = start as u64;
                    let end = end as u64;

                    let title = chapter
                        .get_tags()
                        .and_then(|tags| {
                            tags.get::<gst::tags::Title>()
                                .map(|tag| tag.get().unwrap().to_owned())
                        }).unwrap_or_else(get_default_chapter_title);
                    let iter = self.store.insert_with_values(
                        None,
                        None,
                        &[START_COL, END_COL, TITLE_COL, START_STR_COL, END_STR_COL],
                        &[
                            &start,
                            &end,
                            &title,
                            &Timestamp::format(start, false),
                            &Timestamp::format(end, false),
                        ],
                    );

                    self.boundaries
                        .borrow_mut()
                        .add_chapter(start, end, &title, &iter);
                }
            }
        }

        self.iter = self.store.get_iter_first();

        self.selected_iter = match self.iter {
            Some(ref iter) => {
                if ChapterEntry::get_start(&self.store, iter) == 0 {
                    Some(iter.clone())
                } else {
                    None
                }
            }
            None => None,
        };
    }

    // Iterate over the chapters and apply func to all elements until the last
    // or until func returns false
    //
    // If first_iter is_some, iterate from first_iter
    pub fn for_each<F>(&self, first_iter: Option<gtk::TreeIter>, mut func: F)
    where
        F: FnMut(ChapterEntry<'_>) -> bool, // closure must return true to keep going
    {
        let iter = match first_iter {
            Some(first_iter) => first_iter,
            None => match self.store.get_iter_first() {
                Some(first_iter) => first_iter,
                None => return,
            },
        };

        while func(ChapterEntry::new(&self.store, &iter)) && self.store.iter_next(&iter) {}
    }

    // Update chapter according to the given position
    // Returns (has_changed, prev_selected_iter)
    pub fn update_position(&mut self, position: u64) -> (bool, Option<gtk::TreeIter>) {
        let has_changed = match self.selected_iter {
            Some(ref selected_iter) => {
                if position >= ChapterEntry::get_start(&self.store, selected_iter)
                    && position < ChapterEntry::get_end(&self.store, selected_iter)
                {
                    // regular case: position in current chapter => don't change anything
                    // this check is here to save time in the most frequent case
                    return (false, None);
                }
                // chapter has changed
                true
            }
            None => false,
        };

        let prev_selected_iter = self.selected_iter.take();

        if let Some(iter) = self.iter.take() {
            // not in selected_iter or selected_iter not defined yet => find current chapter
            let mut searching_forward = true;
            while if position >= ChapterEntry::get_start(&self.store, &iter)
                && position < ChapterEntry::get_end(&self.store, &iter)
            {
                // position is in iter
                self.selected_iter = Some(iter.clone());
                self.iter = Some(iter);
                return (true, prev_selected_iter);
            } else if position >= ChapterEntry::get_end(&self.store, &iter) && searching_forward {
                // position is after iter and we were already searching forward
                self.store.iter_next(&iter)
            } else if position < ChapterEntry::get_start(&self.store, &iter) {
                // position before iter
                searching_forward = false;
                if self.store.iter_previous(&iter) {
                    // iter is still valid
                    true
                } else {
                    // before first chapter
                    self.iter = self.store.get_iter_first();
                    return (has_changed, prev_selected_iter);
                }
            } else {
                // in a gap between two chapters
                self.iter = Some(iter);
                return (has_changed, prev_selected_iter);
            } {}

            // passed the end of the last chapter
            // prevent from searching in subsequent calls
            self.iter = None;
        }

        (has_changed, prev_selected_iter)
    }

    pub fn prepare_for_seek(&mut self) {
        if self.iter.is_none() {
            // either there is no chapter or previous position had already passed the end
            // => force scanning of the chapters list in order to set iter back on track
            self.rewind();
        }
    }

    // Returns an iter on the new chapter
    pub fn add_chapter(&mut self, position: u64, duration: u64) -> Option<gtk::TreeIter> {
        let (new_iter, end, end_str) = match self.selected_iter.take() {
            Some(selected_iter) => {
                // a chapter is currently selected
                let (current_start, current_end, current_end_str) = {
                    let selected_chapter = ChapterEntry::new(&self.store, &selected_iter);
                    (
                        selected_chapter.start(),
                        selected_chapter.end(),
                        selected_chapter.end_str(),
                    )
                };

                if current_start != position {
                    // update currently selected chapter end
                    // to match the start of the newly added chapter
                    self.store.set(
                        &selected_iter,
                        &[END_COL, END_STR_COL],
                        &[&position, &Timestamp::format(position, false)],
                    );
                    let new_iter = self.store.insert_after(None, &selected_iter);
                    (new_iter, current_end, current_end_str)
                } else {
                    // attempting to add the new chapter at current position
                    // => restore current state
                    self.selected_iter = Some(selected_iter);
                    return None;
                }
            }
            None => {
                match self.iter.take() {
                    Some(iter) => {
                        // chapters are available, but none is selected:
                        // either position is before the first chapter
                        // or in a gap between two chapters
                        let iter_chapter = ChapterEntry::new(&self.store, &iter);
                        let new_chapter_end = iter_chapter.start();
                        if position > new_chapter_end {
                            panic!(
                                concat!(
                                    "ChapterTreeManager::add_chapter inconsistent position",
                                    " {} with regard to current iter [{}, {}]",
                                ),
                                position,
                                iter_chapter.start(),
                                iter_chapter.end(),
                            );
                        }

                        let new_iter = self.store.insert_before(None, &iter);
                        (new_iter, new_chapter_end, iter_chapter.start_str())
                    }
                    None => {
                        // No chapter in iter:
                        // either position is passed the end of last chapter
                        // or there is no chapter
                        let insert_position = match self.store.get_iter_first() {
                            None =>
                            // No chapter yet => inset at the beginning
                            {
                                0i32
                            }
                            Some(_) =>
                            // store contains chapters => insert at the end
                            {
                                -1i32
                            }
                        };

                        let new_iter = self.store.insert(None, insert_position);
                        (new_iter, duration, Timestamp::format(duration, false))
                    }
                }
            }
        };

        let default_title = get_default_chapter_title();
        self.store.set(
            &new_iter,
            &[TITLE_COL, START_COL, START_STR_COL, END_COL, END_STR_COL],
            &[
                &default_title,
                &position,
                &Timestamp::format(position, false),
                &end,
                &end_str,
            ],
        );

        self.boundaries
            .borrow_mut()
            .add_chapter(position, end, &default_title, &new_iter);

        self.selected_iter = Some(new_iter.clone());
        self.iter = Some(new_iter.clone());

        Some(new_iter)
    }

    // Returns an iter on the chapter which should be selected, if any
    pub fn remove_selected_chapter(&mut self) -> Option<gtk::TreeIter> {
        match self.selected_iter.take() {
            Some(selected_iter) => {
                let prev_iter = selected_iter.clone();
                let (selected_end, next_selected_iter) = {
                    let selected_chapter = ChapterEntry::new(&self.store, &selected_iter);
                    let selected_end = selected_chapter.end();
                    if self.store.iter_previous(&prev_iter) {
                        // a chapter starting before currently selected chapter is available
                        // => update its end with the end of currently selected chapter
                        self.store.set(
                            &prev_iter,
                            &[END_COL, END_STR_COL],
                            &[&selected_end, &selected_chapter.end_str()],
                        );

                        (selected_end, Some(prev_iter))
                    } else {
                        // no chapter before => nothing to select
                        (selected_end, None)
                    }
                };

                let start = ChapterEntry::get_start(&self.store, &selected_iter);
                self.boundaries
                    .borrow_mut()
                    .remove_chapter(start, selected_end);

                self.store.remove(&selected_iter);

                self.selected_iter = next_selected_iter.clone();
                match next_selected_iter {
                    None =>
                    // No chapter before => rewind
                    {
                        self.rewind()
                    }
                    Some(ref next_selected_iter) => self.iter = Some(next_selected_iter.clone()),
                }
                next_selected_iter
            }
            None => None,
        }
    }

    pub fn move_chapter_boundary(&mut self, boundary: u64, to_position: u64) -> bool {
        if boundary == to_position {
            return false;
        }

        let (prev_iter, next_iter) = {
            let boundaries = self.boundaries.borrow();
            boundaries.get(&boundary).map_or((None, None), |chapters| {
                (
                    chapters.prev.as_ref().map(|prev| prev.iter.clone()),
                    chapters.next.as_ref().map(|next| next.iter.clone()),
                )
            })
        };

        if prev_iter.is_none() && next_iter.is_none() {
            return false;
        }

        // prevent moving past previous chapter's start
        let to_position = prev_iter.as_ref().map_or(to_position, |prev_iter| {
            let prev_start = ChapterEntry::get_start(&self.store, prev_iter);
            if to_position > prev_start {
                to_position
            } else {
                boundary
            }
        });

        // prevent moving past next chapter's end
        let to_position = next_iter.as_ref().map_or(to_position, |next_iter| {
            let next_end = ChapterEntry::get_end(&self.store, next_iter);
            if to_position < next_end {
                to_position
            } else {
                boundary
            }
        });

        if to_position != boundary {
            // do the actual move
            if let Some(prev_iter) = prev_iter {
                self.store.set(
                    &prev_iter,
                    &[END_COL, END_STR_COL],
                    &[&to_position, &Timestamp::format(to_position, false)],
                );
            }
            if let Some(next_iter) = next_iter {
                self.store.set(
                    &next_iter,
                    &[START_COL, START_STR_COL],
                    &[&to_position, &Timestamp::format(to_position, false)],
                );
            }

            self.boundaries
                .borrow_mut()
                .move_boundary(boundary, to_position);
            true
        } else {
            // no change
            false
        }
    }

    // FIXME: handle hierarchical Tocs
    pub fn get_toc(&self) -> Option<(gst::Toc, usize)> {
        let mut count = 0;
        match self.store.get_iter_first() {
            Some(iter) => {
                let mut toc_edition = gst::TocEntry::new(gst::TocEntryType::Edition, "");
                loop {
                    count += 1;
                    toc_edition
                        .get_mut()
                        .unwrap()
                        .append_sub_entry(ChapterEntry::new(&self.store, &iter).as_toc_entry());

                    if !self.store.iter_next(&iter) {
                        let mut toc = gst::Toc::new(gst::TocScope::Global);
                        toc.get_mut().unwrap().append_entry(toc_edition);
                        return Some((toc, count));
                    }
                }
            }
            None => None,
        }
    }

    pub fn next_iter(&self) -> Option<gtk::TreeIter> {
        match self.selected_iter.as_ref() {
            Some(selected_iter) => {
                let prev_iter = selected_iter.clone();
                if self.store.iter_next(&prev_iter) {
                    Some(prev_iter)
                } else {
                    // FIXME: with hierarchical tocs, this might be a case where
                    // we should check whether the parent node contains something
                    None
                }
            }
            None => self.store.get_iter_first(),
        }
    }

    pub fn prev_iter(&self) -> Option<gtk::TreeIter> {
        match self.selected_iter.as_ref() {
            Some(selected_iter) => {
                let prev_iter = selected_iter.clone();
                if self.store.iter_previous(&prev_iter) {
                    Some(prev_iter)
                } else {
                    // FIXME: with hierarchical tocs, this might be a case where
                    // we should check whether the parent node contains something
                    None
                }
            }
            None => self.store.get_iter_first().map(|cur_iter| {
                let mut last_iter = cur_iter.clone();
                while self.store.iter_next(&cur_iter) {
                    last_iter = cur_iter.clone();
                }
                last_iter
            }),
        }
    }
}
