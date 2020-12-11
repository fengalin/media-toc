use bitflags::bitflags;

use gettextrs::gettext;
use glib::GString;
use gtk::prelude::*;

use std::{borrow::Cow, cell::RefCell, rc::Rc, string::ToString};

use metadata::{default_chapter_title, Duration, Timestamp4Humans, TocVisitor};
use renderers::Timestamp;

use super::{ChapterTimestamps, ChaptersBoundaries};

const START_COL: u32 = 0;
const END_COL: u32 = 1;
const TITLE_COL: u32 = 2;
const START_STR_COL: u32 = 3;
const END_STR_COL: u32 = 4;

pub struct ChapterIterStart {
    pub iter: gtk::TreeIter,
    pub start: Timestamp,
}

struct ChapterIterEnd {
    iter: gtk::TreeIter,
    end: Timestamp,
}

pub enum PositionStatus {
    ChapterChanged {
        prev_chapter: Option<ChapterIterStart>,
    },
    ChapterNotChanged,
}

impl From<Option<ChapterIterStart>> for PositionStatus {
    fn from(prev_chapter: Option<ChapterIterStart>) -> Self {
        PositionStatus::ChapterChanged { prev_chapter }
    }
}

bitflags! {
    struct ColumnOptions: u32 {
        const NONE = 0b0000_0000;
        const CAN_EXPAND = 0b0000_0001;
        const IS_EDITABLE = 0b0000_0010;
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
            .unwrap()
    }

    fn set_title(&self, title: &str) {
        self.store
            .set_value(&self.iter, TITLE_COL, &glib::Value::from(title));
    }

    pub fn start(&self) -> Timestamp {
        self.store
            .get_value(&self.iter, START_COL as i32)
            .get_some::<u64>()
            .unwrap()
            .into()
    }

    pub fn end(&self) -> Timestamp {
        self.store
            .get_value(&self.iter, END_COL as i32)
            .get_some::<u64>()
            .unwrap()
            .into()
    }

    pub fn timestamps(&self) -> ChapterTimestamps {
        ChapterTimestamps {
            start: self.start(),
            end: self.end(),
        }
    }

    pub fn as_toc_entry(&self) -> gst::TocEntry {
        let mut toc_entry = gst::TocEntry::new(
            gst::TocEntryType::Chapter,
            &format!("{}", self.start().as_u64()),
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

struct ChapterRemovalResult {
    removed_ts: ChapterTimestamps,
    selected_iter: Option<gtk::TreeIter>,
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

    fn selected_timestamps(&self) -> Option<ChapterTimestamps> {
        self.selected_chapter().map(|chapter| chapter.timestamps())
    }

    fn selected_path(&self) -> Option<gtk::TreePath> {
        self.selected
            .as_ref()
            .and_then(|sel_iter| self.store.get_path(sel_iter))
    }

    fn iter_chapter(&self) -> Option<ChapterEntry<'_>> {
        self.iter
            .as_ref()
            .map(|iter| ChapterEntry::new(&self.store, iter))
    }

    fn iter_timestamps(&self) -> Option<ChapterTimestamps> {
        self.iter_chapter().map(|chapter| chapter.timestamps())
    }

    fn new_iter(&self) -> Iter<'_> {
        Iter::new(&self.store)
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

    fn add_unchecked(&self, ts: ChapterTimestamps, title: &str) -> gtk::TreeIter {
        self.store.insert_with_values(
            None,
            None,
            &[START_COL, END_COL, TITLE_COL, START_STR_COL, END_STR_COL],
            &[
                &ts.start.as_u64(),
                &ts.end.as_u64(),
                &title,
                &ts.start.for_humans().to_string(),
                &ts.end.for_humans().to_string(),
            ],
        )
    }

    // Returns an iter on the new chapter
    pub fn add(&mut self, target: Timestamp, duration: Duration) -> Option<ChapterIterEnd> {
        let (new_iter, end, end_str) = match self.selected_timestamps() {
            Some(sel_ts) => {
                assert!(self.selected.is_some());

                if sel_ts.start != target {
                    // update currently selected chapter end
                    // to match the start of the newly added chapter
                    self.store.set(
                        self.selected
                            .as_ref()
                            .expect("inconsistency with selected iter"),
                        &[END_COL, END_STR_COL],
                        &[&target.as_u64(), &target.for_humans().to_string()],
                    );
                    let new_iter = self.store.insert_after(
                        None,
                        Some(
                            self.selected
                                .as_ref()
                                .expect("inconsistency with selected iter"),
                        ),
                    );
                    (new_iter, sel_ts.end, sel_ts.end.for_humans().to_string())
                } else {
                    // attempting to add the new chapter at current position
                    return None;
                }
            }
            None => {
                match self.iter_timestamps() {
                    Some(prev_ts) => {
                        // chapters are available, but none is selected:
                        // either position is before the first chapter
                        // or in a gap between two chapters
                        if target > prev_ts.start {
                            panic!(
                                concat!(
                                    "ChapterTree::add_chapter inconsistent target",
                                    " {} with regard to current iter [{}, {}]",
                                ),
                                target, prev_ts.start, prev_ts.end,
                            );
                        }

                        let new_iter = self.store.insert_before(
                            None,
                            Some(&self.iter.as_ref().expect("inconsistency with iter")),
                        );

                        // prev_start is the new chapter's end
                        (
                            new_iter,
                            prev_ts.start,
                            prev_ts.start.for_humans().to_string(),
                        )
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
                            Timestamp4Humans::from_nano(duration.as_u64()).to_string(),
                        )
                    }
                }
            }
        };

        let default_title = default_chapter_title();
        self.store.set(
            &new_iter,
            &[TITLE_COL, START_COL, START_STR_COL, END_COL, END_STR_COL],
            &[
                &default_title,
                &target.as_u64(),
                &target.for_humans().to_string(),
                &end.as_u64(),
                &end_str,
            ],
        );

        self.selected = Some(new_iter.clone());
        self.iter = Some(new_iter.clone());

        Some(ChapterIterEnd {
            iter: new_iter,
            end,
        })
    }

    // remove selected chapter, update the end of previous chapter if any, select it
    // and return useful information
    fn remove(&mut self) -> Option<ChapterRemovalResult> {
        match self.selected_timestamps() {
            Some(rem_ts) => {
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
                        &[&rem_ts.end.as_u64(), &rem_ts.end.for_humans().to_string()],
                    );
                    self.selected = self.iter.clone();
                }

                self.store.remove(&iter_to_remove);

                if !found_previous {
                    self.iter = self.store.get_iter_first();
                }

                Some(ChapterRemovalResult {
                    removed_ts: rem_ts,
                    selected_iter: self.selected.clone(),
                })
            }
            None => None,
        }
    }

    fn select_by_ts(&mut self, ts: Timestamp) -> PositionStatus {
        let prev_sel_chapter = match self.selected_timestamps() {
            Some(sel_ts) => {
                if ts >= sel_ts.start && ts < sel_ts.end {
                    // regular case: current timestamp in current chapter => don't change anything
                    // this check is here to save time in the most frequent case
                    return PositionStatus::ChapterNotChanged;
                }

                assert!(self.selected.is_some());
                Some(ChapterIterStart {
                    iter: self.selected.take().unwrap(),
                    start: sel_ts.start,
                })
            }
            None => None,
        };

        if self.iter.is_some() {
            // not in selected_iter or selected_iter not defined yet
            // => search for a chapter matching current ts
            let mut searching_forward = true;
            loop {
                let iter_ts = self.iter_timestamps().expect("couldn't get start & end");
                if ts >= iter_ts.start && ts < iter_ts.end {
                    // current timestamp is in current chapter
                    self.selected = self.iter.clone();
                    // ChapterChanged
                    return prev_sel_chapter.into();
                } else if ts >= iter_ts.end && searching_forward {
                    // current timestamp is after iter and we were already searching forward
                    let cur_iter = self.iter.clone();
                    self.next();
                    if self.iter.is_none() {
                        // No more chapter => keep track of last iter:
                        // in case of a seek back, we'll start from here
                        self.iter = cur_iter;
                        break;
                    }
                } else if ts < iter_ts.start {
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
        prev: &Option<ChapterIterStart>,
        next: &Option<ChapterIterEnd>,
    ) {
        let target_str = target.for_humans().to_string();
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
        self.title_renderer = Some(self.add_column(
            treeview,
            &gettext("Title"),
            TITLE_COL,
            ColumnOptions::CAN_EXPAND | ColumnOptions::IS_EDITABLE,
        ));
        self.add_column(
            treeview,
            &gettext("Start"),
            START_STR_COL,
            ColumnOptions::NONE,
        );
        self.add_column(treeview, &gettext("End"), END_STR_COL, ColumnOptions::NONE);
    }

    fn add_column(
        &self,
        treeview: &gtk::TreeView,
        title: &str,
        col_id: u32,
        options: ColumnOptions,
    ) -> gtk::CellRendererText {
        let col = gtk::TreeViewColumn::new();
        col.set_title(title);

        let renderer = gtk::CellRendererText::new();
        renderer.set_property_editable(options.contains(ColumnOptions::IS_EDITABLE));

        col.pack_start(&renderer, true);
        col.add_attribute(&renderer, "text", col_id as i32);
        if options.contains(ColumnOptions::CAN_EXPAND) {
            col.set_min_width(70);
            col.set_expand(true);
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

    pub fn selected_path(&self) -> Option<gtk::TreePath> {
        self.tree.selected_path()
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
            sel_chapter.set_title(new_title);
            let ts = sel_chapter.timestamps();
            self.boundaries.borrow_mut().rename_chapter(ts, new_title);
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
                    let ts = ChapterTimestamps::new_from_u64(start as u64, end as u64);

                    let title = chapter
                        .get_tags()
                        .and_then(|tags| {
                            tags.get::<gst::tags::Title>()
                                .and_then(|tag| tag.get().map(ToString::to_string))
                        })
                        .unwrap_or_else(default_chapter_title);

                    let iter = self.tree.add_unchecked(ts, &title);
                    self.boundaries.borrow_mut().add_chapter(ts, title, &iter);
                }
            }
        }

        self.tree.rewind();
    }

    pub fn iter(&self) -> Iter<'_> {
        self.tree.new_iter()
    }

    // Update chapter according to the given ts
    pub fn update_ts(&mut self, ts: Timestamp) -> PositionStatus {
        self.tree.select_by_ts(ts)
    }

    // Returns an iter on the new chapter
    pub fn add_chapter(&mut self, target: Timestamp, duration: Duration) -> Option<gtk::TreeIter> {
        self.tree.add(target, duration).map(|new_chapter| {
            self.boundaries.borrow_mut().add_chapter(
                ChapterTimestamps::new(target, new_chapter.end),
                &default_chapter_title(),
                &new_chapter.iter,
            );

            new_chapter.iter
        })
    }

    // Returns an iter on the chapter which should be selected, if any
    pub fn remove_selected_chapter(&mut self) -> Option<gtk::TreeIter> {
        self.tree.remove().and_then(|res| {
            self.boundaries.borrow_mut().remove_chapter(res.removed_ts);

            res.selected_iter
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

        let (prev_chapter, next_chapter) = {
            let boundaries = self.boundaries.borrow();
            boundaries.get(&boundary).map_or((None, None), |chapters| {
                (
                    chapters.prev.as_ref().map(|prev| ChapterIterStart {
                        iter: prev.iter.clone(),
                        start: prev.ts.start,
                    }),
                    chapters.next.as_ref().map(|next| ChapterIterEnd {
                        iter: next.iter.clone(),
                        end: next.ts.end,
                    }),
                )
            })
        };

        if prev_chapter.is_none() && next_chapter.is_none() {
            return PositionStatus::ChapterNotChanged;
        }

        // prevent moving past previous chapter's start
        let target = prev_chapter.as_ref().map_or(target, |prev_chapter| {
            if target > prev_chapter.start {
                target
            } else {
                boundary
            }
        });

        // prevent moving past next chapter's end
        let target = next_chapter.as_ref().map_or(target, |next_chapter| {
            if target < next_chapter.end {
                target
            } else {
                boundary
            }
        });

        if target != boundary {
            // do the actual move
            self.tree
                .move_boundary(target, &prev_chapter, &next_chapter);
            self.boundaries.borrow_mut().move_boundary(boundary, target);

            PositionStatus::ChapterChanged { prev_chapter }
        } else {
            PositionStatus::ChapterNotChanged
        }
    }

    // FIXME: handle hierarchical Tocs
    pub fn toc(&self) -> Option<(gst::Toc, usize)> {
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
