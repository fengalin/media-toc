extern crate gtk;
use gtk::prelude::*;

extern crate cairo;

use std::rc::Rc;
use std::cell::RefCell;

use ::media::Context;

use super::{ImageSurface};

pub struct InfoController {
    drawingarea: gtk::DrawingArea,

    title_lbl: gtk::Label,
    artist_lbl: gtk::Label,
    container_lbl: gtk::Label,
    audio_codec_lbl: gtk::Label,
    video_codec_lbl: gtk::Label,
    duration_lbl: gtk::Label,

    chapter_treeview: gtk::TreeView,
    chapter_store: gtk::ListStore,

    thumbnail: Rc<RefCell<Option<ImageSurface>>>,
}

impl InfoController {
    pub fn new(builder: &gtk::Builder) -> Self {
        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let this = InfoController {
            drawingarea: builder.get_object("thumbnail-drawingarea").unwrap(),

            title_lbl: builder.get_object("title-lbl").unwrap(),
            artist_lbl: builder.get_object("artist-lbl").unwrap(),
            container_lbl: builder.get_object("container-lbl").unwrap(),
            audio_codec_lbl: builder.get_object("audio_codec-lbl").unwrap(),
            video_codec_lbl: builder.get_object("video_codec-lbl").unwrap(),
            duration_lbl: builder.get_object("duration-lbl").unwrap(),

            chapter_treeview: builder.get_object("chapter-treeview").unwrap(),
            // columns: Id, Title, Start, End
            chapter_store: gtk::ListStore::new(&[gtk::Type::I32, gtk::Type::String, gtk::Type::String, gtk::Type::String]),

            thumbnail: Rc::new(RefCell::new(None)),
        };

        this.chapter_treeview.set_model(Some(&this.chapter_store));
        this.add_chapter_column("Id", 0, false);
        this.add_chapter_column("Title", 1, true);
        this.add_chapter_column("Start", 2, false);
        this.add_chapter_column("End", 3, false);

        let thumbnail_weak = Rc::downgrade(&this.thumbnail);
        this.drawingarea.connect_draw(move |drawing_area, cairo_ctx| {
            if let Some(thumbnail_rc) = thumbnail_weak.upgrade() {
                let thumbnail_opt = thumbnail_rc.borrow();
                if let Some(ref thumbnail) = *thumbnail_opt {
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

        this
    }

    fn add_chapter_column(&self, title: &str, col_id: i32, can_expand: bool) {
        let col = gtk::TreeViewColumn::new();
        col.set_title(title);
        let renderer = gtk::CellRendererText::new();
        col.pack_start(&renderer, true);
        col.add_attribute(&renderer, "text", col_id);
        if can_expand {
            col.set_min_width(70);
            col.set_expand(can_expand);
        }
        self.chapter_treeview.append_column(&col);
    }

    pub fn new_media(&mut self, ctx: &Context) {
        let mut info = ctx.info.lock()
            .expect("Failed to lock media info in InfoController");

        let mut has_image = false;
        if let Some(thumbnail) = info.thumbnail.take() {
            if let Ok(image) = ImageSurface::from_aligned_image(thumbnail) {
                let mut thumbnail_ref = self.thumbnail.borrow_mut();
                *thumbnail_ref = Some(image);
                has_image = true;
            }
        };

        self.title_lbl.set_label(&info.title);
        self.artist_lbl.set_label(&info.artist);
        // Fix container for mp3 audio files
        let container = if info.video_codec.is_empty()
            && info.audio_codec.to_lowercase().find("mp3").is_some()
        {
            "MP3"
        }
        else {
            &info.container
        };
        self.container_lbl.set_label(container);
        self.audio_codec_lbl.set_label(
            if !info.audio_codec.is_empty() { &info.audio_codec } else { "-" }
        );
        self.video_codec_lbl.set_label(
            if !info.video_codec.is_empty() { &info.video_codec } else { "-" }
        );
        self.duration_lbl.set_label(&format!("{}", ctx.get_duration()));

        if has_image {
            self.drawingarea.show();
            self.drawingarea.queue_draw();
        }
        else {
            self.drawingarea.hide();
        }

        self.chapter_store.clear();
        // FIX for sample.mkv video: generate ids (TODO: remove)
        for (id, chapter) in info.chapters.iter().enumerate() {
            self.chapter_store.insert_with_values(
                None, &[0, 1, 2, 3],
                &[&((id+1) as u32), &chapter.title(), &format!("{}", &chapter.start), &format!("{}", chapter.end)],
            );
        }
    }
}
