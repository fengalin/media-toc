extern crate gtk;
extern crate cairo;

extern crate ffmpeg;

use std::ops::{Deref, DerefMut};

use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;
use cairo::enums::{FontSlant, FontWeight};

use ffmpeg::format::stream::disposition::ATTACHED_PIC;

use ::media::Context;
use ::media::PacketNotifiable;

use super::MediaNotifiable;
use super::MediaController;

fn ffmpeg_pixel_format_to_cairo(ffmpeg_px: ffmpeg::format::Pixel) -> cairo::Format {
    match ffmpeg_px {
        ffmpeg::format::Pixel::ARGB => cairo::Format::ARgb32,
        ffmpeg::format::Pixel::RGB24 => cairo::Format::Rgb24,
        ffmpeg::format::Pixel::RGB565LE => cairo::Format::Rgb16_565,
        _ => cairo::Format::Invalid,
    }
}

pub struct VideoController {
    media_ctl: MediaController,
    drawingarea: gtk::DrawingArea,
    message: String,
    incoming_frame: Option<ffmpeg::frame::Video>,
    frame: Option<ffmpeg::frame::Video>,
    graph: Option<ffmpeg::filter::Graph>,
}


impl VideoController {
    pub fn new(builder: &gtk::Builder) -> Rc<RefCell<VideoController>> {
        // need a RefCell because the callbacks will use immutable versions of vc
        // when the UI controllers will get a mutable version from time to time
        let vc = Rc::new(RefCell::new(VideoController {
            media_ctl: MediaController::new(builder.get_object("video-container").unwrap()),
            drawingarea: builder.get_object("video-drawingarea").unwrap(),
            message: "video place holder".to_owned(),
            incoming_frame: None,
            frame: None,
            graph: None,
        }));

        let vc_for_cb = vc.clone();
        vc.borrow().drawingarea.connect_draw(move |_, cairo_ctx| {
            vc_for_cb.borrow().draw(&cairo_ctx);
            Inhibit(false)
        });

        vc
    }

    fn build_graph(&mut self, decoder: &ffmpeg::codec::decoder::Video) -> Result<bool, String> { // TODO: check how to return Ok() only
        match self.graph {
            Some(_) => (),
            None => {
                let mut graph = ffmpeg::filter::Graph::new();

                let args = format!("width={}:height={}:pix_fmt={}:time_base={}:pixel_aspect={}",
                                   decoder.width(), decoder.height(),
                                   decoder.format().descriptor().unwrap().name(),
                                   decoder.time_base(), decoder.aspect_ratio());

                let in_filter = ffmpeg::filter::find("buffer").unwrap();
                match graph.add(&in_filter, "in", &args) {
                    Ok(_) => (),
                    Err(error) => return Err(format!("Error adding in pad: {:?}", error)),
                }

                let out_filter = ffmpeg::filter::find("buffersink").unwrap();
                match graph.add(&out_filter, "out", "") {
                    Ok(_) => (),
                    Err(error) => return Err(format!("Error adding out pad: {:?}", error)),
                }
                {
                    let mut out_pad = graph.get("out").unwrap();
                    out_pad.set_pixel_format(ffmpeg::format::Pixel::RGB565LE);
                }

                {
                    let in_parser;
                    match graph.output("in", 0) {
                        Ok(value) => in_parser = value,
                        Err(error) => return Err(format!("Error getting output for in pad: {:?}", error)),
                    }
                    let out_parser;
                    match in_parser.input("out", 0) {
                        Ok(value) => out_parser = value,
                        Err(error) => return Err(format!("Error getting input for out pad: {:?}", error)),
                    }
                    match out_parser.parse("copy") {
                        Ok(_) => (),
                        Err(error) => return Err(format!("Error parsing format: {:?}", error)),
                    }
                }

                match graph.validate() {
                    Ok(_) => self.graph = Some(graph),
                    Err(error) => return Err(format!("Error validating graph: {:?}", error)),
                }
            },
        }

        Ok(true)
    }

    fn convert_to_rgb(&mut self, decoder: &ffmpeg::codec::decoder::Video) -> Result<ffmpeg::frame::Video, String> {
        match self.build_graph(decoder) {
            Ok(_) => {
                let mut graph = self.graph.as_mut().unwrap();
                graph.get("in").unwrap().source().add(self.incoming_frame.as_mut().unwrap()).unwrap();

                let mut frame_rgb = ffmpeg::frame::Video::empty();
                while let Ok(..) = graph.get("out").unwrap().sink().frame(&mut frame_rgb) {
                }

                Ok(frame_rgb)
            },
            Err(error) => Err(error),
        }
    }

    fn draw(&self, cr: &cairo::Context) {
        let allocation = self.drawingarea.get_allocation();

        match self.frame {
            Some(ref frame) => {
                let planes = frame.planes();
                if planes > 0 {
                    let surface = cairo::ImageSurface::create_for_data(
                            frame.data(0).to_vec().into_boxed_slice(), |_| {},
                            ffmpeg_pixel_format_to_cairo(frame.format()),
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
                }
            },
            None => {
                cr.scale(allocation.width as f64, allocation.height as f64);
                cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
                cr.set_font_size(0.07);

                cr.move_to(0.1, 0.53);
                cr.show_text(&self.message);
            },
        }
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
    fn new_media(&mut self, context: &mut Context) {
        self.incoming_frame = None;
        self.frame = None;
        self.graph = None;
        self.message = match context.video_stream.as_mut() {
            Some(stream) => {
                self.set_index(stream.index);

                self.show();
                println!("\n** Video stream\n{:?}", &stream);

                let stream_type;
                if stream.disposition | ATTACHED_PIC == ATTACHED_PIC {
                    stream_type = "image";
                }
                else {
                    stream_type = "video stream";
                }
                format!("{} {}", stream_type, self.stream_index().unwrap())
            },
            None => {
                self.hide();
                "no video stream".to_owned()
            },
        };

        self.drawingarea.queue_draw();
    }
}

impl PacketNotifiable for VideoController {
    fn new_packet(&mut self, stream: &ffmpeg::format::stream::Stream,
                  packet: &ffmpeg::codec::packet::Packet) -> Result<bool, String> {
        self.print_packet_content(stream, packet);

        let mut message = String::new();
        let mut got_frame = false;

        let decoder = stream.codec().decoder();
        match decoder.video() {
            Ok(mut video) => {
                {
                    let ref mut incoming_frame = match self.incoming_frame {
                        Some(ref mut frame) => frame,
                        None => {
                            self.incoming_frame = Some(ffmpeg::frame::Video::empty());
                            self.incoming_frame.as_mut().unwrap()
                        },
                    };
                    match video.decode(packet, incoming_frame) {
                        Ok(decode_got_frame) => {
                            got_frame = decode_got_frame;
                            let planes = incoming_frame.planes();
                            println!("\tdecoded video frame, found {} planes - got frame: {}", planes, got_frame);
                            for index in 0..planes {
                                println!("\tplane: {} - data len: {}", index, incoming_frame.data(index).len());
                            }
                        },
                        Err(error) => {
                            message = format!("Error decoding video: {:?}", error);
                        },
                    }
                }

                // TODO: make a stream that allows converting frames as packets arrive
                if got_frame {
                    match self.convert_to_rgb(&video) {
                        Ok(frame_rgb) => self.frame = Some(frame_rgb),
                        Err(error) =>  println!("\tError converting to rgb: {:?}", error),
                    }
                    self.incoming_frame = None;
                }
            },
            Err(error) => {
                message = format!("Error getting video decoder: {:?}", error);
            },
        }

        if message.is_empty() {
            Ok(got_frame)
        }
        else {
            self.incoming_frame = None;
            Err(message)
        }
    }
}
