extern crate ffmpeg;

extern crate chrono;

use std::collections::HashSet;
use std::collections::HashMap;

use std::rc::Weak;
use std::rc::Rc;
use std::cell::RefCell;

use std::path::PathBuf;

use std::fmt;

use chrono::NaiveTime;

use ffmpeg::format::stream::disposition::ATTACHED_PIC;


pub trait VideoNotifiable {
    fn new_video_frame(&mut self, &ffmpeg::frame::Video);
}
pub trait AudioNotifiable {
    fn new_audio_frame(&mut self, &ffmpeg::frame::Audio);
}


pub struct Timestamp {
    timestamp: NaiveTime,
    is_positive: bool,
}

impl Timestamp {
    pub fn new() -> Timestamp {
        Timestamp {
            timestamp: NaiveTime::from_num_seconds_from_midnight(0, 0),
            is_positive: true,
        }
    }

    pub fn from_sec_time_factor(sec: i64, time_factor: f64) -> Timestamp {
        println!("from_sec_time_factor: {}, {}", sec, time_factor);
        let sec_f = sec.abs() as f64 * time_factor;
        Timestamp {
            timestamp: NaiveTime::from_num_seconds_from_midnight(
                sec_f.trunc() as u32,
                (sec_f.fract() * 1_000_000f64) as u32
            ),
            is_positive: if sec >= 0 { true } else { false },
        }
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}",
            if self.is_positive { "".to_owned() } else  { "-".to_owned() },
            self.timestamp.format("%H:%M:%S%.3f").to_string()
        )
    }
}

pub struct Chapter {
    pub id: i32,
    pub start: Timestamp,
    pub end: Timestamp,
    pub metadata: HashMap<String, String>,
}

impl Chapter {
    pub fn new() -> Chapter {
        Chapter{
            id: 0,
            start: Timestamp::new(),
            end: Timestamp::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn set_id(&mut self, id: i32) {
        self.id = id;
    }

    pub fn set_start(&mut self, sec: i64, time_factor: f64) {
        self.start = Timestamp::from_sec_time_factor(sec, time_factor);
    }


    pub fn set_end(&mut self, sec: i64, time_factor: f64) {
        self.end = Timestamp::from_sec_time_factor(sec, time_factor);
    }

    pub fn title(&self) -> &str {
        match self.metadata.get("title") {
            Some(title) => &title,
            None => "",
        }
    }
}

pub struct Context {
    pub ffmpeg_context: ffmpeg::format::context::Input,
    pub file_name: String,
    pub name: String,

    pub artist: String,
    pub title: String,
    pub duration: Timestamp,
    pub description: String,
    pub chapters: Vec<Chapter>,

    pub video_stream: Option<usize>,
    pub video_decoder: Option<ffmpeg::codec::decoder::Video>,
    video_notifiables: Vec<Weak<RefCell<VideoNotifiable>>>,
    pub video_is_thumbnail: bool,

    pub audio_stream: Option<usize>,
    pub audio_decoder: Option<ffmpeg::codec::decoder::Audio>,
    audio_notifiables: Vec<Weak<RefCell<AudioNotifiable>>>,
}


impl Context {
    pub fn new(path: &PathBuf) -> Result<Context, String> {
        println!("\n*** Attempting to open {:?}", path);

        match ffmpeg::format::input(&path.as_path()) {
            Ok(ffmpeg_context) => {
                let mut new_ctx = Context{
                    file_name: String::from(path.file_name().unwrap().to_str().unwrap()),
                    name: String::from(path.file_stem().unwrap().to_str().unwrap()),

                    artist: String::new(),
                    title: String::new(),
                    duration: Timestamp::from_sec_time_factor(
                        ffmpeg_context.duration(),
                        1f64 / ffmpeg::ffi::AV_TIME_BASE as f64
                    ),
                    description: String::new(),
                    chapters: Vec::new(),

                    video_stream: None,
                    video_decoder: None,
                    video_notifiables: Vec::new(),
                    video_is_thumbnail: false,

                    audio_stream: None,
                    audio_decoder: None,
                    audio_notifiables: Vec::new(),
                    ffmpeg_context: ffmpeg_context,
                };

                {
                    let format = new_ctx.ffmpeg_context.format();
                    new_ctx.description = String::from(format.description());

                    new_ctx.init_video_decoder();
                    new_ctx.init_audio_decoder();
                    new_ctx.init_metadata();
                }

                // TODO: see what we should do with subtitles

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

    fn init_video_decoder(&mut self) {
        let stream_index;
        let stream = match self.ffmpeg_context.streams().best(ffmpeg::media::Type::Video) {
            Some(best_stream) => {
                stream_index = best_stream.index();
                if best_stream.disposition() & ATTACHED_PIC == ATTACHED_PIC {
                    self.video_is_thumbnail = true;
                    println!("\nFound thumbnail in stream with id: {}, start time: {}",
                        stream_index, best_stream.start_time()
                    );
                }
                else {
                    self.video_is_thumbnail = false;
                    println!("\nFound video stream with id: {}", stream_index);
                }
	            for (k, v) in best_stream.metadata().iter() {
	                println!("\tmetadata {}: {}", k, v);
	            }
                for side_data in best_stream.side_data() {
                    println!("\tside data {:?}, len: {}", side_data.kind(), side_data.data().len());
                }
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
                    Ok(_) => (),
                    Err(error) => panic!("Failed to set parameters for video decoder: {:?}", error),
                }

                println!("\tvideo decoder: {:?}, time base {}, delay: {}, width: {}, height: {}",
                         decoder.format(), decoder.time_base(), decoder.delay(),
                         decoder.width(), decoder.height()
                 );
                self.video_stream = Some(stream_index);
                self.video_decoder = Some(decoder);
            },
            Err(error) => panic!("Failed to get video decoder: {:?}", error),
        }
    }

    fn init_audio_decoder(&mut self) {
        let stream_index;
        let stream = match self.ffmpeg_context.streams().best(ffmpeg::media::Type::Audio) {
            Some(best_stream) => {
                stream_index = best_stream.index();
                println!("\nFound audio stream with id: {}, start time: {}",
                         stream_index, best_stream.start_time()
                );
	            for (k, v) in best_stream.metadata().iter() {
	                if k.to_lowercase() == "artist" {
	                    self.artist = v.to_owned();
	                }
	                else if k.to_lowercase() == "title" {
	                    self.title = v.to_owned();
	                }
	                println!("\tmetadata {}: {}", k, v);
	            }
                for side_data in best_stream.side_data() {
                    println!("\tside data {:?}, len: {}", side_data.kind(), side_data.data().len());
                }
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
                    Ok(_) => (),
                    Err(error) => panic!("Failed to set parameters for audio decoder: {:?}", error),
                }
                println!("\taudio decoder: {:?}, time base {}, delay: {}, channels: {}, frame count: {}",
                         decoder.format(), decoder.time_base(),
                         decoder.delay(), decoder.channels(), decoder.frames());
                self.audio_stream = Some(stream_index);
                self.audio_decoder = Some(decoder);
            },
            Err(error) => panic!("Failed to get audio decoder: {:?}", error),
        }
    }


    fn init_metadata(&mut self) {
	    for (k, v) in self.ffmpeg_context.metadata().iter() {
	        if k.to_lowercase() == "artist" {
	            self.artist = v.to_owned();
	        }
	        else if k.to_lowercase() == "title" {
	            self.title = v.to_owned();
	        }
	    }
	    if self.title.is_empty() {
            self.title = self.name.clone();
	    }

        for avchapter in self.ffmpeg_context.chapters() {
            let mut chapter = Chapter::new();
            chapter.set_id(avchapter.id());
            let time_base = avchapter.time_base();
            let time_factor = time_base.numerator() as f64 / time_base.denominator() as f64;
            chapter.set_start(avchapter.start(), time_factor);
            chapter.set_end(avchapter.end(), time_factor);
		    for (k, v) in avchapter.metadata().iter() {
			    chapter.metadata.insert(k.to_lowercase(), v.to_owned());
		    }
            self.chapters.push(chapter);
        }
    }

    pub fn register_video_notifiable(&mut self, notifiable: Rc<RefCell<VideoNotifiable>>) {
        self.video_notifiables.push(Rc::downgrade(&notifiable));
    }

    pub fn register_audio_notifiable(&mut self, notifiable: Rc<RefCell<AudioNotifiable>>) {
        self.audio_notifiables.push(Rc::downgrade(&notifiable));
    }

    pub fn preview(&mut self) {
        let mut stream_processed = HashSet::new();
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
                            let mut video = match self.video_decoder {
                                Some(ref mut decoder) => decoder,
                                None => panic!("Error getting video decoder for stream {}", stream_index),
                            };
                            if stream_processed.contains(&stream_index) {
                                // stream already processed
                                continue;
                            }
                            match video.decode(&packet, &mut video_frame) {
                                Ok(decode_got_frame) =>  {
                                    got_frame = decode_got_frame;
                                    println!("\tdecoded video frame, got frame: {}", got_frame);
                                },
                                Err(error) => println!("Error decoding video packet for stream {}: {:?}", stream_index, error),
                            }
                            if got_frame {
                                stream_processed.insert(stream_index);
                                for notifiable_weak in self.video_notifiables.iter() {
                                    match notifiable_weak.upgrade() {
                                        Some(notifiable) => {
                                            notifiable.borrow_mut().new_video_frame(&video_frame);
                                        },
                                        None => (),
                                    };
                                }

                                video_frame = ffmpeg::frame::Video::empty();
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
                            let ref mut audio = match self.audio_decoder.as_mut() {
                                Some(decoder) => decoder,
                                None => panic!("Error getting audio decoder for stream {}", stream_index),
                            };
                            if stream_processed.contains(&stream_index) {
                                // stream already processed
                                continue;
                            }
                            match audio.decode(&packet, &mut audio_frame) {
                                Ok(decode_got_frame) =>  {
                                    got_frame = decode_got_frame;
                                    println!("\tdecoded audio frame, got frame: {}", got_frame);
                                },
                                Err(error) => panic!("Error decoding audio packet for stream {}: {:?}", stream_index, error),
                            }
                            if got_frame {
                                stream_processed.insert(stream_index);
                                for notifiable_weak in self.audio_notifiables.iter() {
                                    match notifiable_weak.upgrade() {
                                        Some(notifiable) => {
                                            notifiable.borrow_mut().new_audio_frame(&audio_frame);
                                        },
                                        None => (),
                                    };
                                }

                                audio_frame = ffmpeg::frame::Audio::empty();
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

            if stream_processed.len() == expected_frames {
                break;
            }
        }
    }
}


fn print_packet_content(stream: &ffmpeg::format::stream::Stream, packet: &ffmpeg::codec::packet::Packet) {
    print!("\n* Packet for {:?} stream: {}", stream.disposition(), stream.index());
    print!(" size: {} - duration: {}, is key: {}",
             packet.size(), packet.duration(), packet.is_key(),
    );
    match packet.pts() {
        Some(pts) => print!(" pts: {}", pts),
        None => (),
    }
    match packet.dts() {
        Some(dts) => print!(" dts: {}", dts),
        None => (),
    }
    println!();
    for side_data in stream.side_data() {
        println!("\tside data {:?}, len: {}", side_data.kind(), side_data.data().len());
    }
}
