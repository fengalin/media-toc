extern crate ffmpeg;

use std::path::Path;
use std::ops::{Deref, DerefMut};

use ffmpeg::Rational;
use ffmpeg::format::stream::Disposition;

pub struct Context {
    ffmpeg_context: ffmpeg::format::context::Input,
    pub name: String,
    pub description: String,
    pub video_stream: Option<VideoStream>,
    pub audio_stream: Option<AudioStream>,
}


impl Context {
    pub fn new(path: &Path) -> Result<Context, String> {
        match ffmpeg::format::input(&path) {
            Ok(ffmpeg_context) => {
                // for the moment, let's work on the best streams, if available
                let mut new_ctx = Context{
                    ffmpeg_context: ffmpeg_context,
                    name: String::new(),
                    description: String::new(),
                    video_stream: None,
                    audio_stream: None,
                };

                let stream_count;
                {
                    let format = new_ctx.ffmpeg_context.format();
                    new_ctx.name = String::from(format.name());
                    new_ctx.description = String::from(format.description());

                    let stream_iter = new_ctx.ffmpeg_context.streams();
                    new_ctx.video_stream = VideoStream::new(&stream_iter);
                    new_ctx.audio_stream = AudioStream::new(&stream_iter);
                    stream_count = stream_iter.size_hint().0;
                }

                // TODO: process metadata

                // TODO: see what we should do with subtitles

                println!("Input: {} - {}, {} streams",
                         &new_ctx.name, &new_ctx.description, stream_count
                );

                if stream_count > 0 {
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

}

// TODO: use lifetimes to hold ffmped stream and codec within the structure below
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
        let new_stream = Stream {
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
        };

        println!("stream index {} - {:?}:", new_stream.index, new_stream.codec_medium);
        println!("\ttime_base: {}", new_stream.time_base);
        println!("\tstart_time: {}", new_stream.start_time);
        println!("\tduration (stream timebase): {}", new_stream.duration);
        println!("\tduration (seconds): {:.2}", new_stream.duration as f64 * f64::from(stream.time_base()));
        println!("\tframes: {}", new_stream.frames);
        println!("\tdisposition: {:?}", new_stream.disposition);
        println!("\tdiscard: {:?}", new_stream.discard);
        println!("\trate: {}", new_stream.rate);
        println!("\tcodec_id: {:?}", new_stream.codec_id);

        new_stream
    }
}

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
                        let new_vs = VideoStream{
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
                        };

                        println!("\tbit_rate: {}", new_vs.bit_rate);
                        println!("\tmax_bit_rate: {}", new_vs.max_bit_rate);
                        println!("\tdelay: {}", new_vs.delay);
					    println!("\twidth: {}", new_vs.width);
					    println!("\theight: {}", new_vs.height);
					    println!("\tformat: {:?}", new_vs.format);
					    println!("\thas_b_frames: {}", new_vs.has_b_frames);
					    println!("\taspect_ratio: {}", new_vs.aspect_ratio);
					    println!("\tcolor_space: {:?}", new_vs.color_space);
					    println!("\tcolor_range: {:?}", new_vs.color_range);
					    println!("\tcolor_primaries: {:?}", new_vs.color_primaries);
					    println!("\tcolor_transfer_characteristic: {:?}", new_vs.color_transfer_characteristic);
					    println!("\tchroma_location: {:?}", new_vs.chroma_location);
					    println!("\treferences: {}", new_vs.references);
                        println!("\tintra_dc_precision: {}", new_vs.intra_dc_precision);

                        Some(new_vs)
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
                        let new_as = AudioStream{
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
                        };

                        println!("\tbit_rate: {}", new_as.bit_rate);
                        println!("\tmax_bit_rate: {}", new_as.max_bit_rate);
                        println!("\tdelay: {}", new_as.delay);
                        println!("\trate: {}", new_as.rate);
                        println!("\tchannels: {}", new_as.channels);
                        println!("\tformat: {:?}", new_as.format);
                        println!("\tframes: {}", new_as.frames);
                        println!("\talign: {}", new_as.align);
                        println!("\tchannel_layout: {:?}", new_as.channel_layout);
                        println!("\tframe_start: {:?}", new_as.frame_start);

                        Some(new_as)
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
