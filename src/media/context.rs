extern crate ffmpeg;

use std::path::Path;

use std::rc::Weak;
use std::rc::Rc;
use std::cell::RefCell;

pub trait VideoNotifiable {
    fn new_video_frame(&mut self, ffmpeg::frame::Video);
}
pub trait AudioNotifiable {
    fn new_audio_frame(&mut self, ffmpeg::frame::Audio);
}

pub struct Context {
    pub ffmpeg_context: ffmpeg::format::context::Input,
    pub name: String,
    pub description: String,

    pub video_stream: Option<usize>,
    pub video_decoder: Option<ffmpeg::codec::decoder::Video>,
    pub video_notifiable: Option<Weak<RefCell<VideoNotifiable>>>,

    pub audio_stream: Option<usize>,
    pub audio_decoder: Option<ffmpeg::codec::decoder::Audio>,
    pub audio_notifiable: Option<Weak<RefCell<AudioNotifiable>>>,
}


fn print_packet_content(stream: &ffmpeg::format::stream::Stream, packet: &ffmpeg::codec::packet::Packet) {
    println!("\n* Packet for {:?} stream: {}", stream.disposition(), stream.index());
    println!("\tsize: {} - duration: {}, is key: {}",
             packet.size(), packet.duration(), packet.is_key(),
    );
    match packet.pts() {
        Some(pts) => println!("\tpts: {}", pts),
        None => (),
    }
    match packet.dts() {
        Some(dts) => println!("\tdts: {}", dts),
        None => (),
    }
    if let Some(data) = packet.data() {
        println!("\tfound data with len: {}", data.len());
    }
    let side_data_iter = stream.side_data();
    let side_data_len = side_data_iter.size_hint().0;
    if side_data_len > 0 {
        println!("\tside data nb: {}", side_data_len);
    }

    let decoder = stream.codec().decoder();
    match decoder.medium() {
        ffmpeg::media::Type::Video => match decoder.video() {
            Ok(video) => println!("\tvideo decoder: {:?}, width: {}, height: {}",
                                  video.format(), video.width(), video.height()),
            Err(_) => (),
        },
        ffmpeg::media::Type::Audio => match decoder.audio() {
            Ok(audio) => println!("\taudio decoder: {:?}, channels: {}, frame count: {}",
                                  audio.format(), audio.channels(), audio.frames()),
            Err(_) => (),
        },
        _ => (),
    }
}



impl Context {
    pub fn new(path: &Path) -> Result<Context, String> {
        match ffmpeg::format::input(&path) {
            Ok(ffmpeg_context) => {
                let mut new_ctx = Context{
                    ffmpeg_context: ffmpeg_context,
                    name: String::new(),
                    description: String::new(),

                    video_stream: None,
                    video_decoder: None,
                    video_notifiable: None,

                    audio_stream: None,
                    audio_decoder: None,
                    audio_notifiable: None,
                };

                {
                    let format = new_ctx.ffmpeg_context.format();
                    new_ctx.name = String::from(format.name());
                    new_ctx.description = String::from(format.description());

                    new_ctx.init_video_decoder();
                    new_ctx.init_audio_decoder();
                }

                // TODO: process metadata

                // TODO: see what we should do with subtitles

                println!("\n*** New media - Input: {} - {}", &new_ctx.name, &new_ctx.description);

                //new_ctx.preview();

                if new_ctx.video_stream.is_some() || new_ctx.audio_stream.is_some() {
                    // TODO: also check for misdetections (detection score)
                    Ok(new_ctx)
                }
                else {
                    Err("Couldn't find any video or audio stream".to_owned())
                }
            },
            Err(error) => {
                Err(format!("{:?}", error))
            },
        }
    }

    pub fn init_video_decoder(&mut self) {
        let stream = match self.ffmpeg_context.streams().best(ffmpeg::media::Type::Video) {
            Some(best_stream) => {
                println!("\nFound video stream with id: {}", best_stream.index());
                best_stream
            },
            None => {
                println!("No video stream");
                return;
            },
        };

        match stream.codec().decoder().video() {
            Ok(mut decoder) => {
                match decoder.set_parameters(stream.parameters()) {
                    Ok(_) => {
                        println!("\tinitialzed video decoder: {:?}", decoder.format());
                    },
                    Err(error) => panic!("Failed to set parameters for video decoder: {:?}", error),
                }

                self.video_stream = Some(stream.index());
                self.video_decoder = Some(decoder);
            },
            Err(error) => panic!("Failed to get video decoder: {:?}", error),
        }
    }

    pub fn init_audio_decoder(&mut self) {
        let stream = match self.ffmpeg_context.streams().best(ffmpeg::media::Type::Audio) {
            Some(best_stream) => {
                println!("\nFound audio stream with id: {}", best_stream.index());
                best_stream
            },
            None => {
                println!("No audio stream");
                return;
            },
        };

        match stream.codec().decoder().audio() {
            Ok(mut decoder) => {
                match decoder.set_parameters(stream.parameters()) {
                    Ok(_) => {
                        println!("\tinitialzed audio decoder: {:?} - channels: {}", decoder.format(), decoder.channels());
                    },
                    Err(error) => panic!("Failed to set parameters for audio decoder: {:?}", error),
                }

                self.audio_stream = Some(stream.index());
                self.audio_decoder = Some(decoder);
            },
            Err(error) => panic!("Failed to get audio decoder: {:?}", error),
        }
    }

    pub fn register_video_notifiable(&mut self, notifiable: Rc<RefCell<VideoNotifiable>>) {
        self.video_notifiable = Some(Rc::downgrade(&notifiable));
    }

    pub fn register_audio_notifiable(&mut self, notifiable: Rc<RefCell<AudioNotifiable>>) {
        self.audio_notifiable = Some(Rc::downgrade(&notifiable));
    }

    pub fn preview(&mut self) {
        let mut frames_processed = 0;
        let mut expected_frames = 0;
        if self.video_stream.is_some() {
            expected_frames += 1;
        }
        if self.audio_stream.is_some() {
            expected_frames += 1;
        }

        let mut video_frame = ffmpeg::frame::Video::empty();
        let mut audio_frame = ffmpeg::frame::Audio::empty();

        for (stream, packet) in self.ffmpeg_context.packets() {
            print_packet_content(&stream, &packet);
            let mut got_frame = false;
            let decoder = stream.codec().decoder();
            match decoder.medium() {
                ffmpeg::media::Type::Video => match self.video_stream {
                    Some(stream_index) => {
                        if stream_index == stream.index() {
                            let mut video = match stream.codec().decoder().video() {
                                Ok(decoder) => decoder,
                                Err(error) => panic!("Error getting video decoder for stream {}: {:?}", stream_index, error),
                            };
                            /*
                            let mut video = match self.video_decoder {
                                Some(ref mut decoder) => decoder,
                                None => panic!("Error getting video decoder for stream {}", stream_index),
                            };
                            */
                            match video.decode(&packet, &mut video_frame) {
                                Ok(decode_got_frame) =>  {
                                    got_frame = decode_got_frame;
                                    let planes = video_frame.planes();
                                    println!("\tdecoded video frame, found {} planes - got frame: {}", planes, got_frame);
                                    for index in 0..planes {
                                        println!("\tplane: {} - data len: {}", index, video_frame.data(index).len());
                                    }
                                },
                                Err(error) => println!("Error decoding video packet for stream {}: {:?}", stream_index, error),
                            }
                            if got_frame {
                                frames_processed += 1;
                                match self.video_notifiable.as_ref() {
                                    Some(notifiable_weak) => {
                                        match notifiable_weak.upgrade() {
                                            Some(notifiable) => {
                                                notifiable.borrow_mut().new_video_frame(video_frame);
                                                video_frame = ffmpeg::frame::Video::empty();
                                            },
                                            None => (),
                                        }
                                    },
                                    None => (),
                                }
                            }
                        }
                        else {
                            println!("Skipping stream {}", stream.index());
                        }
                    },
                    None => panic!("No video decoder"),
                },
                ffmpeg::media::Type::Audio => match self.audio_stream {
                    Some(stream_index) => {
                        if stream_index == stream.index() {
                            let mut audio = match stream.codec().decoder().audio() {
                                Ok(decoder) => decoder,
                                Err(error) => panic!("Error getting audio decoder for stream {}: {:?}", stream_index, error),
                            };
                            /*
                            let mut audio = match self.audio_decoder {
                                Some(ref mut decoder) => decoder,
                                None => panic!("Error getting audio decoder for stream {}", stream_index),
                            };
                            */
                            match audio.decode(&packet, &mut audio_frame) {
                                Ok(decode_got_frame) =>  {
                                    got_frame = decode_got_frame;
                                    let planes = audio_frame.planes();
                                    println!("\tdecoded audio frame, found {} planes - got frame: {}", planes, got_frame);
                                    for index in 0..planes {
                                        println!("\tplane: {} - data len: {}", index, audio_frame.data(index).len());
                                    }
                                },
                                Err(error) => panic!("Error decoding audio packet for stream {}: {:?}", stream_index, error),
                            }
                            if got_frame {
                                frames_processed += 1;
                                match self.audio_notifiable.as_ref() {
                                    Some(notifiable_weak) => {
                                        match notifiable_weak.upgrade() {
                                            Some(notifiable) => {
                                                notifiable.borrow_mut().new_audio_frame(audio_frame);
                                                audio_frame = ffmpeg::frame::Audio::empty();
                                            },
                                            None => (),
                                        }
                                    },
                                    None => (),
                                }
                            }
                        }
                        else {
                            println!("Skipping stream {}", stream.index());
                        }
                    },
                    None => panic!("No audio decoder"),
                },
                _ => (),
            }

            if frames_processed == expected_frames {
                break;
            }
        }
    }
}
