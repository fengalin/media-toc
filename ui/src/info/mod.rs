mod chapters_boundaries;
pub use self::chapters_boundaries::{ChapterTimestamps, ChaptersBoundaries};

mod chapter_tree_manager;
pub use self::chapter_tree_manager::{ChapterEntry, ChapterTreeManager, PositionStatus};

mod controller;
pub use self::controller::Controller;

mod dispatcher;
pub use self::dispatcher::Dispatcher;

use crate::UIEventChannel;
use renderers::Timestamp;

#[derive(Debug)]
pub enum Event {
    AddChapter,
    ChapterClicked(gtk::TreePath),
    Refresh(Timestamp),
    RemoveChapter,
    RenameChapter(String),
    ToggleChapterList(bool),
    ToggleRepeat(bool),
}

fn add_chapter() {
    UIEventChannel::send(Event::AddChapter);
}

fn chapter_clicked(tree_path: gtk::TreePath) {
    UIEventChannel::send(Event::ChapterClicked(tree_path));
}

pub fn refresh(ts: Timestamp) {
    UIEventChannel::send(Event::Refresh(ts));
}

fn rename_chapter(new_title: impl ToString) {
    UIEventChannel::send(Event::RenameChapter(new_title.to_string()));
}

fn remove_chapter() {
    UIEventChannel::send(Event::RemoveChapter);
}

fn toggle_chapter_list(must_show: bool) {
    UIEventChannel::send(Event::ToggleChapterList(must_show));
}

fn toggle_repeat(must_show: bool) {
    UIEventChannel::send(Event::ToggleRepeat(must_show));
}
