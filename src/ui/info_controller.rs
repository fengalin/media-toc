extern crate gtk;
use gtk::prelude::*;

extern crate cairo;

extern crate image;

use std::ops::{Deref, DerefMut};

use ::media::Context;

use super::{MediaController, MediaHandler};

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
    pub fn new(builder: &gtk::Builder) -> Self {
        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let ic = InfoController {
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
        };

        ic.chapter_treeview.set_model(Some(&ic.chapter_store));
        ic.add_chapter_column(&"Id", 0, false);
        ic.add_chapter_column(&"Title", 1, true);
        ic.add_chapter_column(&"Start", 2, false);
        ic.add_chapter_column(&"End", 3, false);

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

        // TODO: move the image decoding in the context
        // so that we can also build an image preview from a video frame
        let mut has_image = false;
        if let Some(thumbnail) = context.thumbnail.as_ref() {
            if let Ok(image) = image::load_from_memory(thumbnail.as_slice()) {
                if let Some(rgb_image) = image.as_rgb8().as_mut() {
                    has_image = true;

                    // Fix stride: there must be a better way...
                    // Cairo uses 4 bytes per pixel, image uses 3 in different order
                    let format = cairo::Format::Rgb24;
                    let width = rgb_image.width() as i32;
                    let height = rgb_image.height() as i32;
                    let stride = width * 4;

                    // TODO: optimize
                    let buffer = rgb_image.to_vec();
                    let mut strided_image: Vec<u8> = Vec::with_capacity((height * stride) as usize);
                    let mut index: usize = 0;
                    for _ in 0..height {
                        for _ in 0..width {
                            strided_image.push(*buffer.get(index+2).unwrap());
                            strided_image.push(*buffer.get(index+1).unwrap());
                            strided_image.push(*buffer.get(index).unwrap());
                            strided_image.push(0);
                            index += 3;
                        }
                    }

                    let surface = cairo::ImageSurface::create_for_data(
                        strided_image.into_boxed_slice(), |_| {}, format,
                        width, height, stride
                    );

                    // TODO: find a way to disconnect previous handler, otherwise
                    // they stack on each other...
                    self.draw_handler = self.drawingarea.connect_draw(move |ref drawing_area, ref cairo_ctx| {
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
                        cairo_ctx.set_source_surface(&surface, x, y);
                        cairo_ctx.paint();
                        Inhibit(false)
                    });
                }
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
