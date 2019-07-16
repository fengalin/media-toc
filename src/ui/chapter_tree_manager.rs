use glib::GString;

use gettextrs::gettext;
use gstreamer as gst;

use glib;

use gtk;
use gtk::prelude::*;

use std::{borrow::Cow, cell::RefCell, rc::Rc, string::ToString};

use media::Timestamp;
use metadata::{get_default_chapter_title, Timestamp4Humans, TocVisitor};

use super::ChaptersBoundaries;

const START_COL: u32 = 0;
const END_COL: u32 = 1;
const TITLE_COL: u32 = 2;
const START_STR_COL: u32 = 3;
const END_STR_COL: u32 = 4;

pub struct PrevChapter {
    pub iter: gtk::TreeIter,
    pub start: Timestamp,
}

pub struct NextChapter {
    pub iter: gtk::TreeIter,
    pub end: Timestamp,
}

pub enum PositionStatus {
    ChapterChanged(Option<PrevChapter>),
    ChapterNotChanged,
}

impl From<Option<PrevChapter>> for PositionStatus {
    fn from(prev_chap: Option<PrevChapter>) -> Self {
        PositionStatus::ChapterChanged(prev_chap)
    }
}

#[derive(Clone)]
pub struct ChapterEntry<'entry> {
    store: &'entry gtk::TreeStore,
    iter: Cow<'entry, gtk::TreeIter>,
}

impl<'entry> ChapterEntry<'entry> {
    fn new(store: &'entry gtk::TreeStore, iter: &'entry gtk::TreeIter) -> ChapterEntry<'entry> {
        ChapterEntry {
            store,
            iter: Cow::Borrowed(iter),
        }
    }

    fn new_owned(store: &'entry gtk::TreeStore, iter: gtk::TreeIter) -> ChapterEntry<'entry> {
        ChapterEntry {
            store,
            iter: Cow::Owned(iter),
        }
    }

    pub fn iter(&self) -> &gtk::TreeIter {
        self.iter.as_ref()
    }

    pub fn title(&self) -> GString {
        self.store
            .get_value(&self.iter, TITLE_COL as i32)
            .get::<GString>()
            .unwrap()
    }

    pub fn start(&self) -> Timestamp {
        self.store
            .get_value(&self.iter, START_COL as i32)
            .get::<u64>()
            .unwrap()
            .into()
    }

    pub fn start_str(&self) -> GString {
        self.store
            .get_value(&self.iter, START_STR_COL as i32)
            .get::<GString>()
            .unwrap()
    }

    pub fn start_ts(&self) -> Timestamp4Humans {
        self.start().get_4_humans()
    }

    pub fn end(&self) -> Timestamp {
        self.store
            .get_value(&self.iter, END_COL as i32)
            .get::<u64>()
            .unwrap()
            .into()
    }

    pub fn end_str(&self) -> GString {
        self.store
            .get_value(&self.iter, END_STR_COL as i32)
            .get::<GString>()
            .unwrap()
    }

    #[allow(dead_code)]
    pub fn end_ts(&self) -> Timestamp4Humans {
        self.end().get_4_humans()
    }

    pub fn as_toc_entry(&self) -> gst::TocEntry {
        let mut toc_entry = gst::TocEntry::new(
            gst::TocEntryType::Chapter,
            &format!("{}", self.start_ts().nano_total),
        );
        toc_entry
            .get_mut()
            .unwrap()
            .set_start_stop_times(self.start().as_i64(), self.end().as_i64());

        let mut tag_list = gst::TagList::new();
        tag_list
            .get_mut()
            .unwrap()
            .add::<gst::tags::Title>(&self.title().as_str(), gst::TagMergeMode::Replace);
        toc_entry.get_mut().unwrap().set_tags(tag_list);

        toc_entry
    }
}

impl Into<gtk::TreeIter> for ChapterEntry<'_> {
    fn into(self) -> gtk::TreeIter {
        self.iter.into_owned()
    }
}

struct ChapterTree {
    store: gtk::TreeStore,
    iter: Option<gtk::TreeIter>,
    selected: Option<gtk::TreeIter>,
}

impl ChapterTree {
    fn new(store: gtk::TreeStore) -> Self {
        ChapterTree {
            store,
            iter: None,
            selected: None,
        }
    }

    fn store(&self) -> &gtk::TreeStore {
        &self.store
    }

    fn clear(&mut self) {
        self.selected = None;
        self.iter = None;
        self.store.clear();
    }

    fn unselect(&mut self) {
        self.selected = None;
    }

    fn rewind(&mut self) {
        self.iter = self.store.get_iter_first();
        self.selected = match &self.iter_chapter() {
            Some(first_chapter) => {
                if first_chapter.start() == Timestamp::default() {
                    self.iter.clone()
                } else {
                    None
                }
            }
            None => None,
        };
    }

    fn chapter_from_path(&self, tree_path: &gtk::TreePath) -> Option<ChapterEntry<'_>> {
        self.store
            .get_iter(tree_path)
            .map(|iter| ChapterEntry::new_owned(&self.store, iter))
    }

    fn selected_chapter(&self) -> Option<ChapterEntry<'_>> {
        self.selected
            .as_ref()
            .map(|selected| ChapterEntry::new(&self.store, selected))
    }

    fn selected_start_end(&self) -> Option<(Timestamp, Timestamp, GString)> {
        self.selected_chapter()
            .map(|chapter| (chapter.start(), chapter.end(), chapter.end_str()))
    }

    fn iter_chapter(&self) -> Option<ChapterEntry<'_>> {
        self.iter
            .as_ref()
            .map(|iter| ChapterEntry::new(&self.store, iter))
    }

    fn iter_start_end(&self) -> Option<(Timestamp, GString, Timestamp)> {
        self.iter_chapter()
            .map(|chapter| (chapter.start(), chapter.start_str(), chapter.end()))
    }

    fn next(&mut self) -> Option<ChapterEntry<'_>> {
        match self.iter.take() {
            Some(iter) => {
                if self.store.iter_next(&iter) {
                    self.iter = Some(iter);
                    let store = &self.store;
                    self.iter
                        .as_ref()
                        .map(|iter| ChapterEntry::new(store, iter))
                } else {
                    None
                }
            }
            None => None,
        }
    }

    fn pick_next(&self) -> Option<ChapterEntry<'_>> {
        match self.selected.as_ref() {
            Some(selected) => {
                let iter = selected.clone();
                if self.store.iter_next(&iter) {
                    Some(ChapterEntry::new_owned(&self.store, iter))
                } else {
                    // FIXME: with hierarchical tocs, this might be a case where
                    // we should check whether the parent node contains something
                    None
                }
            }
            None => self
                .store
                .get_iter_first()
                .map(|first_iter| ChapterEntry::new_owned(&self.store, first_iter)),
        }
    }

    fn previous(&mut self) -> Option<ChapterEntry<'_>> {
        match self.iter.take() {
            Some(iter) => {
                if self.store.iter_previous(&iter) {
                    self.iter = Some(iter);
                    let store = &self.store;
                    self.iter
                        .as_ref()
                        .map(|iter| ChapterEntry::new(store, iter))
                } else {
                    None
                }
            }
            None => None,
        }
    }

    fn pick_previous(&self) -> Option<ChapterEntry<'_>> {
        match self.selected.as_ref() {
            Some(selected) => {
                let prev_iter = selected.clone();
                if self.store.iter_previous(&prev_iter) {
                    Some(ChapterEntry::new_owned(&self.store, prev_iter))
                } else {
                    // FIXME: with hierarchical tocs, this might be a case where
                    // we should check whether the parent node contains something
                    None
                }
            }
            None => self.store.get_iter_first().map(|iter| {
                let mut last_iter = iter.clone();
                while self.store.iter_next(&iter) {
                    last_iter = iter.clone();
                }
                ChapterEntry::new_owned(&self.store, last_iter)
            }),
        }
    }

    fn add_unchecked(&self, start: Timestamp, end: Timestamp, title: &str) -> gtk::TreeIter {
        self.store.insert_with_values(
            None,
            None,
            &[START_COL, END_COL, TITLE_COL, START_STR_COL, END_STR_COL],
            &[
                &start.as_u64(),
                &end.as_u64(),
                &title,
                &start.get_4_humans().as_string(false),
                &end.get_4_humans().as_string(false),
            ],
        )
    }

    // Returns an iter on the new chapter
    pub fn add(&mut self, target: Timestamp, duration: u64) -> Option<NextChapter> {
        let (new_iter, end, end_str) = match self.selected_start_end() {
            Some((sel_start, sel_end, sel_end_str)) => {
                assert!(self.selected.is_some());

                if sel_start != target {
                    // update currently selected chapter end
                    // to match the start of the newly added chapter
                    self.store.set(
                        self.selected
                            .as_ref()
                            .expect("inconsistency with selected iter"),
                        &[END_COL, END_STR_COL],
                        &[&target.as_u64(), &target.get_4_humans().as_string(false)],
                    );
                    let new_iter = self.store.insert_after(
                        None,
                        Some(
                            self.selected
                                .as_ref()
                                .expect("inconsistency with selected iter"),
                        ),
                    );
                    (new_iter, sel_end, sel_end_str)
                } else {
                    // attempting to add the new chapter at current position
                    return None;
                }
            }
            None => {
                match self.iter_start_end() {
                    Some((prev_start, prev_start_str, prev_end)) => {
                        // chapters are available, but none is selected:
                        // either position is before the first chapter
                        // or in a gap between two chapters
                        if target > prev_start {
                            panic!(
                                concat!(
                                    "ChapterTree::add_chapter inconsistent target",
                                    " {} with regard to current iter [{}, {}]",
                                ),
                                target, prev_start, prev_end,
                            );
                        }

                        let new_iter = self.store.insert_before(
                            None,
                            Some(&self.iter.as_ref().expect("inconsistency with iter")),
                        );

                        // prev_start is the new chapter's end
                        (new_iter, prev_start, prev_start_str)
                    }
                    None => {
                        // No chapter in iter:
                        // either the chapter to add timestamp is passed the end of last chapter
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
                        (
                            new_iter,
                            duration.into(),
                            Timestamp4Humans::format(duration, false).into(),
                        )
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
                &target.as_u64(),
                &target.get_4_humans().as_string(false),
                &end.as_u64(),
                &end_str,
            ],
        );

        self.selected = Some(new_iter.clone());
        self.iter = Some(new_iter.clone());

        // FIXME: might need a better name
        Some(NextChapter {
            iter: new_iter,
            end,
        })
    }

    // remove selected chapter, update the end of previous chapter if any, select it and return it
    // FIXME: find something better for the return value
    fn remove(&mut self) -> Option<(Timestamp, Timestamp, Option<gtk::TreeIter>)> {
        match self.selected_start_end() {
            Some((rem_start, rem_end, rem_end_str)) => {
                assert!(self.iter.is_some());
                assert!(self.selected.is_some());

                let iter_to_remove = self
                    .selected
                    .take()
                    .expect("inconsistency with selected iter");

                let found_previous = self
                    .store
                    .iter_previous(self.iter.as_ref().expect("inconsistency with iter"));
                if found_previous {
                    self.store.set(
                        self.iter.as_ref().expect("inconsistency with iter"),
                        &[END_COL, END_STR_COL],
                        &[&rem_end.as_u64(), &rem_end_str],
                    );
                    self.selected = self.iter.clone();
                }

                self.store.remove(&iter_to_remove);

                if !found_previous {
                    self.iter = self.store.get_iter_first();
                }

                Some((
                    rem_start,
                    rem_end,
                    self.selected.clone().map(|selected| selected),
                ))
            }
            None => None,
        }
    }

    fn select_by_ts(&mut self, ts: Timestamp) -> PositionStatus {
        let prev_sel_chapter = match self.selected_start_end() {
            Some((start, end, _end_str)) => {
                if ts >= start && ts < end {
                    // regular case: current timestamp in current chapter => don't change anything
                    // this check is here to save time in the most frequent case
                    return PositionStatus::ChapterNotChanged;
                }

                assert!(self.selected.is_some());
                Some(PrevChapter {
                    iter: self.selected.take().unwrap(),
                    start,
                })
            }
            None => None,
        };

        if self.iter.is_some() {
            // not in selected_iter or selected_iter not defined yet
            // => search for a chapter matching current ts
            let mut searching_forward = true;
            loop {
                let (start, _start_str, end) =
                    self.iter_start_end().expect("couldn't get start & end");
                if ts >= start && ts < end {
                    // current timestamp is in current chapter
                    self.selected = self.iter.clone();
                    // ChapterChanged
                    return prev_sel_chapter.into();
                } else if ts >= end && searching_forward {
                    // current timestamp is after iter and we were already searching forward
                    let cur_iter = self.iter.clone();
                    self.next();
                    if self.iter.is_none() {
                        // No more chapter => keep track of last iter:
                        // in case of a seek back, we'll start from here
                        self.iter = cur_iter;
                        break;
                    }
                } else if ts < start {
                    // current timestamp before iter
                    searching_forward = false;
                    self.previous();
                    if self.iter.is_none() {
                        // before first chapter
                        self.iter = self.store.get_iter_first();
                        // ChapterChanged
                        return prev_sel_chapter.into();
                    }
                } else {
                    // in a gap between two chapters
                    break;
                }
            }
        }

        // Couldn't find a chapter to select
        // consider that the chapter changed only if a chapter was selected before
        match prev_sel_chapter {
            Some(prev_sel_chapter) => Some(prev_sel_chapter).into(),
            None => PositionStatus::ChapterNotChanged,
        }
    }

    fn move_boundary(
        &self,
        target: Timestamp,
        prev: &Option<PrevChapter>,
        next: &Option<NextChapter>,
    ) {
        let target_str = target.get_4_humans().as_string(false);
        if let Some(prev) = prev {
            self.store.set(
                &prev.iter,
                &[END_COL, END_STR_COL],
                &[&target.as_u64(), &target_str],
            );
        }
        if let Some(next) = next {
            self.store.set(
                &next.iter,
                &[START_COL, START_STR_COL],
                &[&target.as_u64(), &target_str],
            );
        }
    }
}

pub struct ChapterTreeManager {
    tree: ChapterTree,
    pub title_renderer: Option<gtk::CellRendererText>,
    boundaries: Rc<RefCell<ChaptersBoundaries>>,
}

impl ChapterTreeManager {
    pub fn new(store: gtk::TreeStore, boundaries: Rc<RefCell<ChaptersBoundaries>>) -> Self {
        ChapterTreeManager {
            tree: ChapterTree::new(store),
            title_renderer: None,
            boundaries,
        }
    }

    pub fn init_treeview(&mut self, treeview: &gtk::TreeView) {
        treeview.set_model(Some(self.tree.store()));
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
            let store_clone = self.tree.store().clone();
            renderer.connect_edited(move |_, tree_path, value| {
                if let Some(iter) = store_clone.get_iter(&tree_path) {
                    // FIXME: can we dot that in rename_selected?
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

    pub fn selected(&self) -> Option<ChapterEntry<'_>> {
        self.tree.selected_chapter()
    }

    pub fn chapter_from_path(&self, tree_path: &gtk::TreePath) -> Option<ChapterEntry<'_>> {
        self.tree.chapter_from_path(tree_path)
    }

    pub fn unselect(&mut self) {
        self.tree.unselect();
    }

    pub fn clear(&mut self) {
        self.tree.clear();
        self.boundaries.borrow_mut().clear();
    }

    pub fn rename_selected(&mut self, new_title: &str) {
        if let Some(sel_chapter) = self.tree.selected_chapter() {
            let (start, end) = (sel_chapter.start(), sel_chapter.end());
            self.boundaries
                .borrow_mut()
                .rename_chapter(start, end, new_title);
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
                    let start = Timestamp::new(start as u64);
                    let end = Timestamp::new(end as u64);

                    let title = chapter
                        .get_tags()
                        .and_then(|tags| {
                            tags.get::<gst::tags::Title>()
                                .and_then(|tag| tag.get().map(ToString::to_string))
                        })
                        .unwrap_or_else(get_default_chapter_title);

                    let iter = self.tree.add_unchecked(start, end, &title);
                    self.boundaries.borrow_mut().add_chapter(
                        start.into(),
                        end.into(),
                        title,
                        &iter,
                    );
                }
            }
        }

        self.tree.rewind();
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter::new(&self.tree.store)
    }

    // Update chapter according to the given ts
    pub fn update_ts(&mut self, ts: Timestamp) -> PositionStatus {
        self.tree.select_by_ts(ts)
    }

    // Returns an iter on the new chapter
    pub fn add_chapter(&mut self, target: Timestamp, duration: u64) -> Option<gtk::TreeIter> {
        self.tree.add(target, duration).map(|new_chapter| {
            self.boundaries.borrow_mut().add_chapter(
                target,
                new_chapter.end,
                &get_default_chapter_title(),
                &new_chapter.iter,
            );

            new_chapter.iter
        })
    }

    // Returns an iter on the chapter which should be selected, if any
    pub fn remove_selected_chapter(&mut self) -> Option<gtk::TreeIter> {
        self.tree
            .remove()
            .map_or(None, |(prev_sel_start, prev_sel_end, new_sel_iter)| {
                self.boundaries
                    .borrow_mut()
                    .remove_chapter(prev_sel_start, prev_sel_end);

                new_sel_iter
            })
    }

    pub fn move_chapter_boundary(
        &mut self,
        boundary: Timestamp,
        target: Timestamp,
    ) -> PositionStatus {
        if boundary == target {
            return PositionStatus::ChapterNotChanged;
        }

        let (prev, next) = {
            let boundaries = self.boundaries.borrow();
            boundaries.get(&boundary).map_or((None, None), |chapters| {
                (
                    chapters.prev.as_ref().map(|prev| PrevChapter {
                        iter: prev.iter.clone(),
                        start: prev.start,
                    }),
                    chapters.next.as_ref().map(|next| NextChapter {
                        iter: next.iter.clone(),
                        end: next.end,
                    }),
                )
            })
        };

        if prev.is_none() && next.is_none() {
            return PositionStatus::ChapterNotChanged;
        }

        // prevent moving past previous chapter's start
        let target = prev.as_ref().map_or(target, |prev| {
            if target > prev.start {
                target
            } else {
                boundary
            }
        });

        // prevent moving past next chapter's end
        let target =
            next.as_ref().map_or(
                target,
                |next| {
                    if target < next.end {
                        target
                    } else {
                        boundary
                    }
                },
            );

        if target != boundary {
            // do the actual move
            self.tree.move_boundary(target, &prev, &next);
            self.boundaries.borrow_mut().move_boundary(boundary, target);

            PositionStatus::ChapterChanged(prev)
        } else {
            PositionStatus::ChapterNotChanged
        }
    }

    // FIXME: handle hierarchical Tocs
    pub fn get_toc(&self) -> Option<(gst::Toc, usize)> {
        let mut count = 0;
        let mut toc_edition = gst::TocEntry::new(gst::TocEntryType::Edition, "");
        for chapter in self.iter() {
            count += 1;
            toc_edition
                .get_mut()
                .unwrap()
                .append_sub_entry(chapter.as_toc_entry());
        }

        if count > 0 {
            let mut toc = gst::Toc::new(gst::TocScope::Global);
            toc.get_mut().unwrap().append_entry(toc_edition);
            Some((toc, count))
        } else {
            None
        }
    }

    pub fn pick_next(&self) -> Option<ChapterEntry<'_>> {
        self.tree.pick_next()
    }

    pub fn pick_previous(&self) -> Option<ChapterEntry<'_>> {
        self.tree.pick_previous()
    }
}

pub struct Iter<'store> {
    store: &'store gtk::TreeStore,
    iter: Option<gtk::TreeIter>,
    is_first: bool,
}

impl<'store> Iter<'store> {
    fn new(store: &'store gtk::TreeStore) -> Self {
        Iter {
            store,
            iter: None,
            is_first: true,
        }
    }
}

impl<'store> Iterator for Iter<'store> {
    type Item = ChapterEntry<'store>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.is_first {
            if let Some(iter) = self.iter.as_mut() {
                if !self.store.iter_next(iter) {
                    self.iter = None;
                }
            }
        } else {
            self.iter = self.store.get_iter_first();
            self.is_first = false;
        }

        self.iter
            .clone()
            .map(|iter| ChapterEntry::new_owned(&self.store, iter))
    }
}
