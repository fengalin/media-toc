extern crate ffmpeg;

use std::path::Path;
use std::ops::{Deref, DerefMut};
use std::collections::HashMap;

use ffmpeg::Rational;
use ffmpeg::format::stream::Disposition;

type PacketCallback = fn(&ffmpeg::format::stream::Stream, &ffmpeg::codec::packet::Packet);

pub struct Context {
    pub ffmpeg_context: ffmpeg::format::context::Input,
    pub name: String,
    pub description: String,
    pub stream_count: usize,
    pub video_stream: Option<VideoStream>,
    pub audio_stream: Option<AudioStream>,
    pub packet_cb_map: HashMap<usize, PacketCallback>,
}


impl Context {
    pub fn new(path: &Path) -> Result<Context, String> {
        match ffmpeg::format::input(&path) {
            Ok(ffmpeg_context) => {
                let mut new_ctx = Context{
                    ffmpeg_context: ffmpeg_context,
                    name: String::new(),
                    description: String::new(),
                    stream_count: 0,
                    video_stream: None,
                    audio_stream: None,
                    packet_cb_map: HashMap::new(),
                };

                {
                    let format = new_ctx.ffmpeg_context.format();
                    new_ctx.name = String::from(format.name());
                    new_ctx.description = String::from(format.description());

                    let stream_iter = new_ctx.ffmpeg_context.streams();
                    new_ctx.video_stream = VideoStream::new(&stream_iter);
                    new_ctx.audio_stream = AudioStream::new(&stream_iter);
                    new_ctx.stream_count = stream_iter.size_hint().0;
                }

                // TODO: process metadata

                // TODO: see what we should do with subtitles

                println!("*** New media - Input: {} - {}, {} streams",
                         &new_ctx.name, &new_ctx.description, new_ctx.stream_count
                );

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

    // FIXME: this is not appropriate for a closure, should use Box<FnMut(,)>
    pub fn register_packets(&mut self, stream_index: usize, cb: PacketCallback) {
        self.packet_cb_map.insert(stream_index, cb);
    }

    pub fn preview(&mut self) {
        let packet_iter = self.ffmpeg_context.packets();
        let mut count = 0;
        for (stream, packet) in packet_iter {
            match self.packet_cb_map.get(&stream.index()) {
                Some(packet_cb) => packet_cb(&stream, &packet),
                None => println!("No handler for stream {}", stream.index()),
            }

            // TODO: do something better like using a HashMap to store
            // streams index that were already previewed
            count += 1;
            if count == self.stream_count {
                break;
            }
        }
    }
}

// TODO: use lifetimes to hold ffmped stream and codec within the structure below

#[derive(Debug)]
pub struct Stream {
    pub index: usize,
    pub time_base: Rational,
    pub start_time: i64,
    pub duration: i64,
    pub frames: i64,
    pub disposition: Disposition,
    pub rate: Rational,
    pub discard: ffmpeg::Discard,
    pub avg_frame_rate: Rational,
    pub codec_medium: ffmpeg::media::Type,
    pub codec_id: ffmpeg::codec::Id,
}

impl Stream {
    pub fn new(stream: ffmpeg::format::stream::Stream) -> Stream {
        let codec = stream.codec();
        Stream {
            index: stream.index(),
            time_base: stream.time_base(),
            start_time: stream.start_time(),
            duration: stream.duration(),
            frames: stream.frames(),
            disposition: stream.disposition(),
            discard: stream.discard(),
            rate: stream.rate(),
            avg_frame_rate: stream.avg_frame_rate(),
            codec_medium: codec.medium(),
            codec_id: codec.id(),
        }
    }
}

#[derive(Debug)]
pub struct VideoStream {
    pub stream: Stream,
	pub bit_rate: usize,
	pub max_bit_rate: usize,
	pub delay: usize,
    width: u32,
    height: u32,
    format: ffmpeg::format::Pixel,
    has_b_frames: bool,
    aspect_ratio: ffmpeg::Rational,
    color_space: ffmpeg::color::Space,
    color_range: ffmpeg::color::Range,
    color_primaries: ffmpeg::color::Primaries,
    color_transfer_characteristic: ffmpeg::color::TransferCharacteristic,
    chroma_location: ffmpeg::chroma::Location,
    references: usize,
    intra_dc_precision: u8,
}

impl VideoStream {
    pub fn new(stream_iter: &ffmpeg::format::context::common::StreamIter) -> Option<VideoStream> {
        match stream_iter.best(ffmpeg::media::Type::Video) {
            Some(video_stream) => {
                match video_stream.codec().decoder().video() {
                    Ok(video) => {
                        Some(VideoStream{
                            bit_rate: video.bit_rate(),
                            max_bit_rate: video.max_bit_rate(),
                            delay: video.delay(),
                            width: video.width(),
                            height: video.height(),
                            format: video.format(),
                            has_b_frames: video.has_b_frames(),
                            aspect_ratio: video.aspect_ratio(),
                            color_space: video.color_space(),
                            color_range: video.color_range(),
                            color_primaries: video.color_primaries(),
                            color_transfer_characteristic: video.color_transfer_characteristic(),
                            chroma_location: video.chroma_location(),
                            references: video.references(),
                            intra_dc_precision: video.intra_dc_precision(),
                            stream: Stream::new(video_stream),
                        })
                    },
                    Err(error) => {
                        println!("video stream: {:?}", error);
                        // TODO: should probably panic here
                        None
                    }
                }
            },
            None => None,
        }
    }
}

impl Deref for VideoStream {
	type Target = Stream;

	fn deref(&self) -> &Self::Target {
		&self.stream
	}
}

impl DerefMut for VideoStream {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.stream
	}
}



#[derive(Debug)]
pub struct AudioStream {
    pub stream: Stream,
	pub bit_rate: usize,
	pub max_bit_rate: usize,
	pub delay: usize,
	pub rate: u32,
	pub channels: u16,
	pub format: ffmpeg::format::Sample,
	pub frames: usize,
	pub align: usize,
	pub channel_layout: ffmpeg::ChannelLayout,
    pub frame_start: Option<usize>,
}

impl AudioStream {
    pub fn new(stream_iter: &ffmpeg::format::context::common::StreamIter) -> Option<AudioStream> {
        match stream_iter.best(ffmpeg::media::Type::Audio) {
            Some(audio_stream) => {
                match audio_stream.codec().decoder().audio() {
                    Ok(audio) => {
                        Some(AudioStream{
                            bit_rate: audio.bit_rate(),
                            max_bit_rate: audio.max_bit_rate(),
                            delay: audio.delay(),
                            rate: audio.rate(),
                            channels: audio.channels(),
                            format: audio.format(),
                            frames: audio.frames(),
                            align: audio.align(),
                            channel_layout: audio.channel_layout(),
                            frame_start: audio.frame_start(),
                            stream: Stream::new(audio_stream),
                        })
                    },
                    Err(error) => {
                        println!("audio stream: {:?}", error);
                        // TODO: should probably panic here
                        None
                    }
                }
            },
            None => None,
        }
   }
}

impl Deref for AudioStream {
	type Target = Stream;

	fn deref(&self) -> &Self::Target {
		&self.stream
	}
}

impl DerefMut for AudioStream {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.stream
	}
}
