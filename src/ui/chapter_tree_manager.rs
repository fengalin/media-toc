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

    fn next(mut self) -> Option<Self> {
        if self.store.iter_next(self.iter.to_mut()) {
            Some(self)
        } else {
            None
        }
    }

    fn previous(mut self) -> Option<Self> {
        if self.store.iter_previous(self.iter.to_mut()) {
            Some(self)
        } else {
            None
        }
    }

    // remove current, update the end of previous chapter if any and return it
    fn remove(self) -> Option<Self> {
        let cur_end = self.end();
        let cur_end_str = self.end_str();
        let cur_iter = self.iter.clone();

        let store = self.store.clone();

        let prev = self.previous();
        if let Some(prev) = &prev {
            prev.set(&[END_COL, END_STR_COL], &[&cur_end.as_u64(), &cur_end_str]);
        }

        store.remove(&cur_iter);

        prev
    }

    fn set(&self, cols: &[u32], values: &[&dyn glib::ToValue]) {
        self.store.set(&self.iter, cols, values);
    }
}

impl Into<gtk::TreeIter> for ChapterEntry<'_> {
    fn into(self) -> gtk::TreeIter {
        self.iter.into_owned()
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

    pub fn get_selected(&self) -> Option<ChapterEntry<'_>> {
        self.selected_iter
            .as_ref()
            .map(|selected_iter| ChapterEntry::new(&self.store, selected_iter))
    }

    fn take_selected(&mut self) -> Option<ChapterEntry<'_>> {
        self.selected_iter
            .take()
            .map(move |selected_iter| ChapterEntry::new_owned(&self.store, selected_iter))
    }

    fn get_iter_chapter(&self) -> Option<ChapterEntry<'_>> {
        self.iter
            .as_ref()
            .map(|iter| ChapterEntry::new(&self.store, iter))
    }

    fn take_iter_chapter(&mut self) -> Option<ChapterEntry<'_>> {
        self.iter
            .take()
            .map(move |iter| ChapterEntry::new_owned(&self.store, iter))
    }

    pub fn get_chapter_from_path(&self, tree_path: &gtk::TreePath) -> Option<ChapterEntry<'_>> {
        self.store
            .get_iter(tree_path)
            .map(|iter| ChapterEntry::new_owned(&self.store, iter))
    }

    pub fn unselect(&mut self) {
        self.selected_iter = None;
    }

    pub fn clear(&mut self) {
        self.selected_iter = None;
        self.iter = None;
        self.boundaries.borrow_mut().clear();
        self.store.clear();
    }

    pub fn rename_selected_chapter(&mut self, new_title: &str) {
        if let Some(cur_chapter) = self.get_selected() {
            let (start, end) = (cur_chapter.start(), cur_chapter.end());
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
                    let start = start as u64;
                    let end = end as u64;

                    let title = chapter
                        .get_tags()
                        .and_then(|tags| {
                            tags.get::<gst::tags::Title>()
                                .and_then(|tag| tag.get().map(ToString::to_string))
                        })
                        .unwrap_or_else(get_default_chapter_title);
                    let iter = self.store.insert_with_values(
                        None,
                        None,
                        &[START_COL, END_COL, TITLE_COL, START_STR_COL, END_STR_COL],
                        &[
                            &start,
                            &end,
                            &title,
                            &Timestamp4Humans::format(start, false),
                            &Timestamp4Humans::format(end, false),
                        ],
                    );

                    self.boundaries.borrow_mut().add_chapter(
                        start.into(),
                        end.into(),
                        title,
                        &iter,
                    );
                }
            }
        }

        self.iter = self.store.get_iter_first();

        self.selected_iter = match &self.get_iter_chapter() {
            Some(chapter) => {
                if chapter.start() == Timestamp::default() {
                    Some(chapter.iter().clone())
                } else {
                    None
                }
            }
            None => None,
        };
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter::new(&self.store)
    }

    // Update chapter according to the given ts
    pub fn update_ts(&mut self, ts: Timestamp) -> PositionStatus {
        let prev_sel_chapter = match self.get_selected() {
            Some(sel_chapter) => {
                let start = sel_chapter.start();
                if ts >= start && ts < sel_chapter.end() {
                    // regular case: current timestamp in current chapter => don't change anything
                    // this check is here to save time in the most frequent case
                    return PositionStatus::ChapterNotChanged;
                }

                assert!(self.selected_iter.is_some());
                Some(PrevChapter {
                    iter: self.selected_iter.take().unwrap(),
                    start,
                })
            }
            None => None,
        };

        if let Some(mut iter_chapter) = self.take_iter_chapter() {
            // not in selected_iter or selected_iter not defined yet
            // => search for a chapter matching current ts
            let mut searching_forward = true;
            loop {
                if ts >= iter_chapter.start() && ts < iter_chapter.end() {
                    // current timestamp is in current chapter
                    let iter: gtk::TreeIter = iter_chapter.into();
                    self.selected_iter = Some(iter.clone());
                    self.iter = Some(iter);
                    // ChapterChanged
                    return prev_sel_chapter.into();
                } else if ts >= iter_chapter.end() && searching_forward {
                    // current timestamp is after iter and we were already searching forward
                    let current_iter = iter_chapter.iter().clone();
                    match iter_chapter.next() {
                        Some(next_chapter) => iter_chapter = next_chapter,
                        None => {
                            // No more chapter => keep track of last iter
                            // In case of a seek back, we'll start from here
                            self.iter = Some(current_iter);
                            break;
                        }
                    }
                } else if ts < iter_chapter.start() {
                    // current timestamp before iter
                    searching_forward = false;
                    match iter_chapter.previous() {
                        Some(prev_chapter) => iter_chapter = prev_chapter,
                        None => {
                            // before first chapter
                            self.iter = self.store.get_iter_first();
                            // ChapterChanged
                            return prev_sel_chapter.into();
                        }
                    }
                } else {
                    // in a gap between two chapters
                    self.iter = Some(iter_chapter.into());
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

    // Returns an iter on the new chapter
    pub fn add_chapter(&mut self, target: Timestamp, duration: u64) -> Option<gtk::TreeIter> {
        let (new_iter, end, end_str) = match self.take_selected() {
            Some(cur_chapter) => {
                // a chapter is currently selected
                let (cur_start, cur_end, cur_end_str) = (
                    cur_chapter.start(),
                    cur_chapter.end(),
                    cur_chapter.end_str(),
                );

                if cur_start != target {
                    // update currently selected chapter end
                    // to match the start of the newly added chapter
                    // FIXME: this could be done in a new method `ChapterEntry::insert_after`
                    cur_chapter.set(
                        &[END_COL, END_STR_COL],
                        &[&target.as_u64(), &target.get_4_humans().as_string(false)],
                    );
                    let cur_iter = cur_chapter.into();
                    let new_iter = self.store.insert_after(None, Some(&cur_iter));
                    (new_iter, cur_end, cur_end_str)
                } else {
                    // attempting to add the new chapter at current position
                    // => restore current state
                    self.selected_iter = Some(cur_chapter.into());
                    return None;
                }
            }
            None => {
                match self.take_iter_chapter() {
                    Some(iter_chapter) => {
                        // chapters are available, but none is selected:
                        // either position is before the first chapter
                        // or in a gap between two chapters
                        let new_chapter_end = iter_chapter.start();
                        if target > new_chapter_end {
                            panic!(
                                concat!(
                                    "ChapterTreeManager::add_chapter inconsistent target",
                                    " {} with regard to current iter [{}, {}]",
                                ),
                                target,
                                iter_chapter.start(),
                                iter_chapter.end(),
                            );
                        }

                        let start_str = iter_chapter.start_str();
                        let iter = iter_chapter.into();
                        // FIXME: this could be done in a new method `ChapterEntry::insert_after`
                        let new_iter = self.store.insert_before(None, Some(&iter));
                        (new_iter, new_chapter_end, start_str)
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

        self.boundaries
            .borrow_mut()
            .add_chapter(target, end, &default_title, &new_iter);

        self.selected_iter = Some(new_iter.clone());
        self.iter = Some(new_iter.clone());

        Some(new_iter)
    }

    // Returns an iter on the chapter which should be selected, if any
    pub fn remove_selected_chapter(&mut self) -> Option<gtk::TreeIter> {
        match self.take_selected() {
            Some(sel_chapter) => {
                let sel_start = sel_chapter.start();
                let sel_end = sel_chapter.end();
                let next_sel_iter: Option<gtk::TreeIter> = sel_chapter
                    .remove()
                    .map(|next_sel_chapter| next_sel_chapter.into());

                match next_sel_iter {
                    Some(ref next_sel_iter) => {
                        self.selected_iter = Some(next_sel_iter.clone());
                        self.iter = Some(next_sel_iter.clone());
                    }
                    None => {
                        // No chapter before => rewind
                        self.selected_iter = None;
                        self.iter = self.store.get_iter_first();
                    }
                }

                self.boundaries
                    .borrow_mut()
                    .remove_chapter(sel_start, sel_end);

                next_sel_iter
            }
            None => None,
        }
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
            if let Some(prev) = &prev {
                self.store.set(
                    &prev.iter,
                    &[END_COL, END_STR_COL],
                    &[&target.as_u64(), &target.get_4_humans().as_string(false)],
                );
            }
            if let Some(next) = &next {
                self.store.set(
                    &next.iter,
                    &[START_COL, START_STR_COL],
                    &[&target.as_u64(), &target.get_4_humans().as_string(false)],
                );
            }

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
        match self.selected_iter.as_ref() {
            Some(selected_iter) => {
                let iter = selected_iter.clone();
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

    pub fn pick_previous(&self) -> Option<ChapterEntry<'_>> {
        match self.selected_iter.as_ref() {
            Some(selected_iter) => {
                let prev_iter = selected_iter.clone();
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
}

pub struct Iter<'store> {
    cur_chapter: Option<ChapterEntry<'store>>,
    is_first: bool,
    store: &'store gtk::TreeStore,
}

impl<'store> Iter<'store> {
    fn new(store: &'store gtk::TreeStore) -> Self {
        Iter {
            cur_chapter: store
                .get_iter_first()
                .map(|iter| ChapterEntry::new_owned(&store, iter)),
            is_first: true,
            store,
        }
    }
}

impl<'store> Iterator for Iter<'store> {
    type Item = ChapterEntry<'store>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(cur_chapter) = &self.cur_chapter {
            if !self.is_first {
                if !self.store.iter_next(cur_chapter.iter()) {
                    self.cur_chapter = None;
                }
            } else {
                self.is_first = false;
            }
        }

        self.cur_chapter.clone()
    }
}
