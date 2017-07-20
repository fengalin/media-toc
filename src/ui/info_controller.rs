extern crate gtk;
extern crate cairo;

extern crate ffmpeg;

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;
use cairo::enums::{FontSlant, FontWeight};

use ::media::Context;
use ::media::VideoNotifiable;

use super::MediaNotifiable;
use super::MediaController;

pub struct InfoController {
    media_ctl: MediaController,
    thumbnail_area: gtk::DrawingArea,
    thumbnail_frame: Option<ffmpeg::frame::Video>,
    graph: Option<ffmpeg::filter::Graph>,
    title_lbl: gtk::Label,
    artist_lbl: gtk::Label,
    description_lbl: gtk::Label,
    duration_lbl: gtk::Label,
}

impl InfoController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<InfoController>> {
        // need a RefCell because the callbacks will use immutable versions of ac
        // when the UI controllers will get a mutable version from time to time
        let ic = Rc::new(RefCell::new(InfoController {
            media_ctl: MediaController::new(builder.get_object("info-box").unwrap()),
            thumbnail_area: builder.get_object("thumbnail-drawingarea").unwrap(),
            thumbnail_frame: None,
            graph: None,
            title_lbl: builder.get_object("title-lbl").unwrap(),
            artist_lbl: builder.get_object("artist-lbl").unwrap(),
            description_lbl: builder.get_object("description-lbl").unwrap(),
            duration_lbl: builder.get_object("duration-lbl").unwrap(),
        }));

        let ic_for_cb = ic.clone();
        ic.borrow().thumbnail_area.connect_draw(move |ref drawing_area, ref cairo_ctx| {
            ic_for_cb.borrow().draw(drawing_area, cairo_ctx);
            Inhibit(false)
        });

        ic
    }

    // TODO: find a way to factorize with VideoController
    // build_graph, convert_to_rgb and draw
    fn build_graph(&mut self, decoder: &ffmpeg::codec::decoder::Video) {
        let mut graph = ffmpeg::filter::Graph::new();

        let args = format!("width={}:height={}:pix_fmt={}:time_base={}:pixel_aspect={}",
                           decoder.width(), decoder.height(),
                           decoder.format().descriptor().unwrap().name(),
                           decoder.time_base(), decoder.aspect_ratio());

        let in_filter = ffmpeg::filter::find("buffer").unwrap();
        match graph.add(&in_filter, "in", &args) {
            Ok(_) => (),
            Err(error) => panic!("Error adding in pad: {:?}", error),
        }

        let out_filter = ffmpeg::filter::find("buffersink").unwrap();
        match graph.add(&out_filter, "out", "") {
            Ok(_) => (),
            Err(error) => panic!("Error adding out pad: {:?}", error),
        }
        {
            let mut out_pad = graph.get("out").unwrap();
            out_pad.set_pixel_format(ffmpeg::format::Pixel::RGB565LE);
        }

        {
            let in_parser;
            match graph.output("in", 0) {
                Ok(value) => in_parser = value,
                Err(error) => panic!("Error getting output for in pad: {:?}", error),
            }
            let out_parser;
            match in_parser.input("out", 0) {
                Ok(value) => out_parser = value,
                Err(error) => panic!("Error getting input for out pad: {:?}", error),
            }
            match out_parser.parse("copy") {
                Ok(_) => (),
                Err(error) => panic!("Error parsing format: {:?}", error),
            }
        }

        match graph.validate() {
            Ok(_) => self.graph = Some(graph),
            Err(error) => panic!("Error validating graph: {:?}", error),
        }
    }

    fn convert_to_rgb(&mut self, frame: &ffmpeg::frame::Video) -> Result<ffmpeg::frame::Video, String> {
        let mut graph = self.graph.as_mut().unwrap();
        graph.get("in").unwrap().source().add(&frame).unwrap();

        let mut frame_rgb = ffmpeg::frame::Video::empty();
        while let Ok(..) = graph.get("out").unwrap().sink().frame(&mut frame_rgb) {
        }

        Ok(frame_rgb)
    }

    fn draw(&self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) {
        let allocation = drawing_area.get_allocation();

        match self.thumbnail_frame {
            Some(ref frame) => {
                let planes = frame.planes();
                if planes > 0 {
                    let pixel_format = match frame.format() {
                        ffmpeg::format::Pixel::ARGB => cairo::Format::ARgb32,
                        ffmpeg::format::Pixel::RGB24 => cairo::Format::Rgb24,
                        ffmpeg::format::Pixel::RGB565LE => cairo::Format::Rgb16_565,
                        _ => cairo::Format::Invalid,
                    };

                    let surface = cairo::ImageSurface::create_for_data(
                            frame.data(0).to_vec().into_boxed_slice(), |_| {},
                            pixel_format,
                            frame.width() as i32, frame.height() as i32,
                            frame.stride(0) as i32
                        );

                    let scale;
                    let alloc_ratio = allocation.width as f64 / allocation.height as f64;
                    let surface_ratio = surface.get_width() as f64 / surface.get_height() as f64;
                    if surface_ratio < alloc_ratio {
                        scale = allocation.height as f64 / surface.get_height() as f64;
                    }
                    else {
                        scale = allocation.width as f64 / surface.get_width() as f64;
                    }
                    let x = (allocation.width as f64 / scale - surface.get_width() as f64).abs() / 2f64;

                    cr.scale(scale, scale);
                    cr.set_source_surface(&surface, x, 0f64);
                    cr.paint();
                }
            },
            None => {
                cr.scale(allocation.width as f64, allocation.height as f64);
                cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
                cr.set_font_size(0.07);

                cr.move_to(0.1, 0.53);
                cr.show_text("thumbnail placeholder");
            },
        }
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

impl MediaNotifiable for InfoController {
    fn new_media(&mut self, context: &Context) {
        self.thumbnail_frame = None;
        self.graph = None;

        // TODO: display metadata

        match context.video_decoder.as_ref() {
            Some(decoder) => {
                self.build_graph(decoder);
                self.thumbnail_area.show();

                self.title_lbl.set_label(&context.title);
                self.artist_lbl.set_label(&context.artist);
                self.description_lbl.set_label(&context.description);
                self.duration_lbl.set_label(&format!("{:.2} s", context.duration));
            },
            None => self.thumbnail_area.hide(),
        };

        self.show();
    }
}

impl VideoNotifiable for InfoController {
    fn new_video_frame(&mut self, frame: &ffmpeg::frame::Video) {
        match self.thumbnail_frame {
            Some(_) => (),
            None => {
                match self.convert_to_rgb(frame) {
                    Ok(frame_rgb) => {
                        self.thumbnail_frame = Some(frame_rgb);
                        self.thumbnail_area.queue_draw();
                    },
                    Err(error) =>  panic!("\tError converting to rgb: {:?}", error),
                }
            },
        };
    }
}
