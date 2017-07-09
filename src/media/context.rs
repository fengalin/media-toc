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
                    stream_count = stream_iter.size_hint().0;

                    new_ctx.video_stream = VideoStream::new(&stream_iter);
                    new_ctx.audio_stream = AudioStream::new(&stream_iter);
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

pub struct Stream {
    pub index: usize,
    pub time_base: Rational,
    pub start_time: i64,
    pub duration: i64,
    pub frames: i64,
    pub disposition: Disposition,
    pub rate: Rational,
    pub avg_frame_rate: Rational,
}

impl Stream {
    pub fn new(stream_iter: &ffmpeg::format::context::common::StreamIter,
               media_type: ffmpeg::media::Type) -> Option<Stream> {
        let stream = stream_iter.best(media_type);
        match stream {
            Some(stream) => {
                Some(Stream {
                    index: stream.index(),
                    time_base: stream.time_base(),
                    start_time: stream.start_time(),
                    duration: stream.duration(),
                    frames: stream.frames(),
                    disposition: stream.disposition(),
                    rate: stream.rate(),
                    avg_frame_rate: stream.avg_frame_rate(),
                })
            },
            None => None,
        }
    }
}

pub struct VideoStream {
    pub stream: Stream,
}

impl VideoStream {
    pub fn new(stream_iter: &ffmpeg::format::context::common::StreamIter) -> Option<VideoStream> {
        let video_stream = Stream::new(stream_iter, ffmpeg::media::Type::Video);

        match video_stream {
            Some(video_stream) => {
                if video_stream.frames > 0 {
                    Some(VideoStream{ stream: video_stream, })
                }
                else {
                    None
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
}

impl AudioStream {
    pub fn new(stream_iter: &ffmpeg::format::context::common::StreamIter) -> Option<AudioStream> {
        let audio_stream = Stream::new(stream_iter, ffmpeg::media::Type::Audio);

        match audio_stream {
            Some(audio_stream) => {
                if audio_stream.frames > 0 || audio_stream.duration > 0 {
                    Some(AudioStream{ stream: audio_stream, })
                }
                else {
                    None
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
