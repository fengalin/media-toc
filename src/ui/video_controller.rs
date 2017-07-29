extern crate gtk;
extern crate cairo;

extern crate gstreamer;

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;

use ::media::Context;
use ::media::VideoNotifiable;

use super::MediaNotifiable;
use super::MediaController;


pub struct VideoController {
    media_ctl: MediaController,
    video_area: gtk::DrawingArea,
    is_thumbnail_only: bool,
    /*frame: Option<ffmpeg::frame::Video>,
    graph: Option<ffmpeg::filter::Graph>, */
}


impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<VideoController>> {
        // need a RefCell because the callbacks will use immutable versions of vc
        // when the UI controllers will get a mutable version from time to time
        let vc = Rc::new(RefCell::new(VideoController {
            media_ctl: MediaController::new(builder.get_object("video-container").unwrap()),
            video_area: builder.get_object("video-drawingarea").unwrap(),
            is_thumbnail_only: false,
            /*frame: None,
            graph: None, */
        }));

        let vc_for_cb = vc.clone();
        vc.borrow().video_area.connect_draw(move |ref drawing_area, ref cairo_ctx| {
            vc_for_cb.borrow().draw(drawing_area, cairo_ctx);
            Inhibit(false)
        });

        vc
    }

    /*
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
    */

    /*
    fn convert_to_rgb(&mut self, frame: &ffmpeg::frame::Video) -> Result<ffmpeg::frame::Video, String> {
        let mut graph = self.graph.as_mut().unwrap();
        graph.get("in").unwrap().source().add(&frame).unwrap();

        let mut frame_rgb = ffmpeg::frame::Video::empty();
        while let Ok(..) = graph.get("out").unwrap().sink().frame(&mut frame_rgb) {
        }

        Ok(frame_rgb)
    }
    */

    fn draw(&self, drawing_area: &gtk::DrawingArea, cr: &cairo::Context) {
        /*
        match self.frame {
            Some(ref frame) => {
                let allocation = drawing_area.get_allocation();
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
                let y = (allocation.height as f64 / scale - surface.get_height() as f64).abs() / 2f64;

                cr.scale(scale, scale);
                cr.set_source_surface(&surface, x, y);
                cr.paint();
            },
            None => (),
        }
        */
    }
}

impl Deref for VideoController {
	type Target = MediaController;

	fn deref(&self) -> &Self::Target {
		&self.media_ctl
	}
}

impl DerefMut for VideoController {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.media_ctl
	}
}

impl MediaNotifiable for VideoController {
    fn new_media(&mut self, context: &Context) {
        /*
        self.frame = None;
        self.graph = None;

        match context.video_decoder.as_ref() {
            Some(decoder) => {
                self.build_graph(decoder);
                self.is_thumbnail_only = context.video_is_thumbnail;
                if !self.is_thumbnail_only {
                    self.show();
                }
                else {
                    self.hide();
                }
            },
            None => {
                self.hide();
            }
        };
        */
    }
}

impl VideoNotifiable for VideoController {
    /*
    fn new_video_frame(&mut self, frame: &ffmpeg::frame::Video) {
        match self.convert_to_rgb(frame) {
            Ok(frame_rgb) => {
                if !self.is_thumbnail_only {
                    self.frame = Some(frame_rgb);
                    self.video_area.queue_draw();
                }
            },
            Err(error) =>  panic!("\tError converting to rgb: {:?}", error),
        }
    }
    */
}
