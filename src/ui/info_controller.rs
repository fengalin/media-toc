extern crate gtk;
extern crate cairo;

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;

use ::media::Context;
use ::media::{MediaHandler, VideoHandler};

use super::MediaController;

pub struct InfoController {
    media_ctl: MediaController,

    title_lbl: gtk::Label,
    artist_lbl: gtk::Label,
    description_lbl: gtk::Label,
    duration_lbl: gtk::Label,

    chapter_treeview: gtk::TreeView,
    chapter_store: gtk::ListStore,
}

impl InfoController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<InfoController>> {
        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let ic = Rc::new(RefCell::new(InfoController {
            media_ctl: MediaController::new(
                builder.get_object("info-box").unwrap(),
                builder.get_object("thumbnail-drawingarea").unwrap()
            ),

            title_lbl: builder.get_object("title-lbl").unwrap(),
            artist_lbl: builder.get_object("artist-lbl").unwrap(),
            description_lbl: builder.get_object("description-lbl").unwrap(),
            duration_lbl: builder.get_object("duration-lbl").unwrap(),

            chapter_treeview: builder.get_object("chapter-treeview").unwrap(),
            // columns: Id, Title, Start, End
            chapter_store: gtk::ListStore::new(&[gtk::Type::I32, gtk::Type::String, gtk::Type::String, gtk::Type::String]),
        }));

        {
            let ic_bor = ic.borrow();
            ic_bor.chapter_treeview.set_model(Some(&ic_bor.chapter_store));
            ic_bor.add_chapter_column(&"Id", 0, false);
            ic_bor.add_chapter_column(&"Title", 1, true);
            ic_bor.add_chapter_column(&"Start", 2, false);
            ic_bor.add_chapter_column(&"End", 3, false);
        }

        ic
    }

    fn add_chapter_column(&self, title: &str, col_id: i32, can_expand: bool) {
        let col = gtk::TreeViewColumn::new();
        col.set_title(title);
        let renderer = gtk::CellRendererText::new();
        col.pack_start(&renderer, true);
        col.add_attribute(&renderer, "text", col_id);
        col.set_expand(can_expand);
        self.chapter_treeview.append_column(&col);
    }
}

impl Deref for InfoController {
	type Target = MediaController;

	fn deref(&self) -> &Self::Target {
		&self.media_ctl
	}
}

impl DerefMut for InfoController {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.media_ctl
	}
}

impl MediaHandler for InfoController {
    fn new_media(&mut self, context: &Context) {
        self.title_lbl.set_label(&context.title);
        self.artist_lbl.set_label(&context.artist);
        self.description_lbl.set_label(&context.description);
        self.duration_lbl.set_label(&format!("{}", context.duration));

        self.chapter_store.clear();
        // FIX for sample.mkv video: generate ids (TODO: remove)
        let mut id = 0;
        for chapter in context.chapters.iter() {
            id += 1;
            self.chapter_store.insert_with_values(
                None, &[0, 1, 2, 3],
                &[&id, &chapter.title(), &format!("{}", &chapter.start), &format!("{}", chapter.end)],
            );
        }
        self.show();
    }
}

impl VideoHandler for InfoController {
    fn new_video_stream(&mut self, context: &mut Context) {
    }

    fn new_video_frame(&mut self, context: &Context) {
    }
}
