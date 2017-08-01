extern crate gtk;
use gtk::prelude::*;

extern crate cairo;

use std::rc::Rc;
use std::cell::{Ref, RefCell};

use std::ops::{Deref, DerefMut};

use ::media::Context;

use super::{ImageSurface, MediaController, MediaHandler};

pub struct InfoController {
    media_ctl: MediaController,

    title_lbl: gtk::Label,
    artist_lbl: gtk::Label,
    description_lbl: gtk::Label,
    duration_lbl: gtk::Label,

    chapter_treeview: gtk::TreeView,
    chapter_store: gtk::ListStore,

    thumbnail: Rc<RefCell<Option<ImageSurface>>>,
}

impl InfoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let mut ic = InfoController {
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

            thumbnail: Rc::new(RefCell::new(None)),
        };

        ic.chapter_treeview.set_model(Some(&ic.chapter_store));
        ic.add_chapter_column(&"Id", 0, false);
        ic.add_chapter_column(&"Title", 1, true);
        ic.add_chapter_column(&"Start", 2, false);
        ic.add_chapter_column(&"End", 3, false);

        let thumbnail_weak = Rc::downgrade(&ic.thumbnail);
        ic.draw_handler = ic.drawingarea.connect_draw(move |ref drawing_area, ref cairo_ctx| {
            if let Some(thumbnail_rc) = thumbnail_weak.upgrade() {
                let thumbnail_ref = thumbnail_rc.borrow();
                if let Some(ref thumbnail) = *thumbnail_ref {
                    let surface = &thumbnail.surface;

                    let allocation = drawing_area.get_allocation();
                    let alloc_ratio = allocation.width as f64 / allocation.height as f64;
                    let surface_ratio = surface.get_width() as f64 / surface.get_height() as f64;
                    let scale = if surface_ratio < alloc_ratio {
                        allocation.height as f64 / surface.get_height() as f64
                    }
                    else {
                        allocation.width as f64 / surface.get_width() as f64
                    };
                    let x = (allocation.width as f64 / scale - surface.get_width() as f64).abs() / 2f64;
                    let y = (allocation.height as f64 / scale - surface.get_height() as f64).abs() / 2f64;

                    cairo_ctx.scale(scale, scale);
                    cairo_ctx.set_source_surface(surface, x, y);
                    cairo_ctx.paint();
                }
            }

            Inhibit(false)
        });

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

        let mut has_image = false;
        if let Some(thumbnail) = context.thumbnail.as_ref() {
            if let Ok(image) = ImageSurface::from_uknown_buffer(thumbnail.as_slice()) {
                let mut thumbnail_ref = self.thumbnail.borrow_mut();
                *thumbnail_ref = Some(image);
                has_image = true;
            }
        };

        if has_image {
            self.drawingarea.show();
            self.drawingarea.queue_draw();
        }
        else {
            self.drawingarea.hide();
        }

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
