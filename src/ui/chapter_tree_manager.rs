extern crate gtk;
use gtk::prelude::*;

use toc::{Chapter, Timestamp};

const START_COL: i32 = 0;
const END_COL: i32 = 1;
const TITLE_COL: i32 = 2;
const START_STR_COL: i32 = 3;
const END_STR_COL: i32 = 4;

pub struct ChapterEntry<'a> {
    store: &'a gtk::TreeStore,
    iter: &'a gtk::TreeIter,
}

impl<'a> ChapterEntry<'a> {
    pub fn new(store: &'a gtk::TreeStore, iter: &'a gtk::TreeIter) -> ChapterEntry<'a> {
        ChapterEntry {
            store: store,
            iter: iter,
        }
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

    pub fn end_ts(&self) -> Timestamp {
        Timestamp::from_nano(ChapterEntry::get_end(self.store, self.iter))
    }

    pub fn get_title(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> String {
        store.get_value(iter, TITLE_COL).get::<String>().unwrap()
    }

    pub fn get_start(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> u64 {
        store.get_value(iter, START_COL).get::<u64>().unwrap()
    }

    pub fn get_start_str(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> String {
        store.get_value(iter, START_STR_COL).get::<String>().unwrap()
    }

    pub fn get_end(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> u64 {
        store.get_value(iter, END_COL).get::<u64>().unwrap()
    }

    pub fn get_end_str(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> String {
        store.get_value(iter, END_STR_COL).get::<String>().unwrap()
    }
}

pub struct ChapterTreeManager {
    store: gtk::TreeStore,
    iter: Option<gtk::TreeIter>,
    selected_iter: Option<gtk::TreeIter>,
}

impl ChapterTreeManager {
    pub fn new_from(store: gtk::TreeStore) -> Self {
        ChapterTreeManager {
            store: store,
            iter: None,
            selected_iter: None,
        }
    }

    pub fn init_treeview(&self, treeview: &gtk::TreeView) {
        treeview.set_model(Some(&self.store));
        self.add_column(treeview, "Title", TITLE_COL, true, true);
        self.add_column(treeview, "Start", START_STR_COL, false, false);
        self.add_column(treeview, "End", END_STR_COL, false, false);
    }

    fn add_column(&self,
        treeview: &gtk::TreeView,
        title: &str,
        col_id: i32,
        can_expand: bool,
        is_editable: bool,
    ) {
        let col = gtk::TreeViewColumn::new();
        col.set_title(title);

        let renderer = gtk::CellRendererText::new();
        if is_editable {
            renderer.set_property_editable(true);
            let store_clone = self.store.clone();
            renderer.connect_edited(move |_, tree_path, value| if let Some(iter) =
                store_clone.get_iter(&tree_path)
            {
                store_clone.set_value(&iter, TITLE_COL as u32, &gtk::Value::from(value));
            });
        }

        col.pack_start(&renderer, true);
        col.add_attribute(&renderer, "text", col_id);
        if can_expand {
            col.set_min_width(70);
            col.set_expand(can_expand);
        }
        treeview.append_column(&col);
    }

    pub fn get_selected_iter(&self) -> Option<gtk::TreeIter> {
        self.selected_iter.clone()
    }

    pub fn get_chapter_at_iter<'a: 'b, 'b>(&'a self,
        iter: &'a gtk::TreeIter
    ) -> ChapterEntry<'b> {
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
        self.store.clear();
    }

    pub fn replace_with(&mut self, chapter_list: &[Chapter]) {
        self.clear();

        for chapter in chapter_list.iter() {
            self.store.insert_with_values(
                None,
                None,
                &[
                    START_COL as u32,
                    END_COL as u32,
                    TITLE_COL as u32,
                    START_STR_COL as u32,
                    END_STR_COL as u32,
                ],
                &[
                    &chapter.start.nano_total,
                    &chapter.end.nano_total,
                    &chapter.title(),
                    &format!("{}", &chapter.start),
                    &format!("{}", chapter.end),
                ],
            );
        }

        self.iter = self.store.get_iter_first();

        self.selected_iter =
            match self.iter {
                Some(ref iter) =>
                    if ChapterEntry::get_start(&self.store, iter) == 0 {
                        Some(iter.clone())
                    } else {
                        None
                    },
                None => None,
            };
    }

    // Iterate over the chapters and apply func to all elements until the last
    // or until func returns false
    //
    // If first_iter is_some, iterate from first_iter
    pub fn for_each<F>(&self, first_iter: Option<gtk::TreeIter>, mut func: F)
    where
        F: FnMut(ChapterEntry) -> bool // closure must return true to keep going
    {
        let iter =
            match first_iter {
                Some(first_iter) => first_iter,
                None =>
                    match self.store.get_iter_first() {
                        Some(first_iter) => first_iter,
                        None => return,
                    },
            };

        while func(ChapterEntry::new(&self.store, &iter)) &&
            self.store.iter_next(&iter) {}
    }

    // Update chapter according to the given position
    // Returns (has_changed, prev_selected_iter)
    pub fn update_position(&mut self, position: u64) -> (bool, Option<gtk::TreeIter>) {
        let has_changed =
            match self.selected_iter {
                Some(ref selected_iter) => {
                    if position >= ChapterEntry::get_start(&self.store, selected_iter) &&
                        position < ChapterEntry::get_end(&self.store, selected_iter)
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
            while if position >= ChapterEntry::get_start(&self.store, &iter) &&
                        position < ChapterEntry::get_end(&self.store, &iter)
                {
                    // position is in iter
                    self.selected_iter = Some(iter.clone());
                    self.iter = Some(iter);
                    return (true, prev_selected_iter);
                } else if position >= ChapterEntry::get_end(&self.store, &iter) &&
                            searching_forward
                {
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
                }
            {}

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
        let (new_iter, end, end_str) =
            match self.selected_iter.take() {
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
                            &[END_COL as u32, END_STR_COL as u32],
                            &[&position, &Timestamp::format(position, false)],
                        );

                        // insert new chapter after current
                        let new_iter = self.store.insert_after(None, &selected_iter);
                        (new_iter, current_end, current_end_str)
                    } else {
                        // attempting to add the new chapter at current position
                        return None;
                    }
                }
                None =>
                    match self.iter.take() {
                        Some(iter) => {
                            // chapters are available, but none is selected:
                            // either position is before the first chapter
                            // or in a gap between two chapters
                            let iter_chapter = ChapterEntry::new(&self.store, &iter);
                            let start = iter_chapter.start();
                            if position > start {
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

                            // iter is next chapter

                            (
                                self.store.insert_before(None, &iter),
                                start,
                                iter_chapter.start_str(),
                            )
                        }
                        None => {
                            // No chapter in iter:
                            // either position is passed the end of last chapter
                            // or there is no chapter
                            let insert_position =
                                match self.store.get_iter_first() {
                                    None => // No chapter yet => inset at the beginning
                                        0i32,
                                    Some(_) => // store contains chapters => insert at the end
                                        -1i32,
                                };
                            (
                                self.store.insert(None, insert_position),
                                duration,
                                Timestamp::format(duration, false),
                            )
                        }
                    }
            };

        self.store.set(
            &new_iter,
            &[START_COL as u32, START_STR_COL as u32, END_COL as u32, END_STR_COL as u32],
            &[&position, &Timestamp::format(position, false), &end, &end_str],
        );

        self.selected_iter = Some(new_iter.clone());
        self.iter = Some(new_iter.clone());
        Some(new_iter)
    }

    // Returns an iter on the chapter which should be selected, if any
    pub fn remove_selected_chapter(&mut self) -> Option<gtk::TreeIter> {
        match self.selected_iter.take() {
            Some(selected_iter) => {
                let prev_iter = selected_iter.clone();
                let next_selected_iter =
                    if self.store.iter_previous(&prev_iter) {
                        // a previous chapter is available => update its end
                        // with the end of currently selected chapter
                        let selected_chapter = ChapterEntry::new(&self.store, &selected_iter);
                        self.store.set(
                            &prev_iter,
                            &[END_COL as u32, END_STR_COL as u32],
                            &[&selected_chapter.end(), &selected_chapter.end_str()],
                        );

                        Some(prev_iter)
                    } else {
                        // no chapter before => nothing to select
                        None
                    };

                self.store.remove(&selected_iter);
                self.selected_iter = next_selected_iter.clone();
                match next_selected_iter {
                    None =>
                        // No chapter before => rewind
                        self.rewind(),
                    Some(ref next_selected_iter) => self.iter = Some(next_selected_iter.clone()),
                }
                next_selected_iter
            }
            None => None,
        }
    }
}
