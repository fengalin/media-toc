extern crate ffmpeg;

use std::path::Path;

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
                    Err(String::from("Couldn't find any video or audio stream"))
                }
            },
            Err(error) => {
                Err(format!("{:?}", error))
            },
        }
    }

}

pub struct VideoStream {
    pub index: usize,
    pub time_base: Rational,
    pub start_time: i64,
    pub duration: i64,
    pub frames: i64,
    pub disposition: Disposition,
    pub rate: Rational,
    pub avg_frame_rate: Rational,
}

impl VideoStream {
    pub fn new(stream_iter: &ffmpeg::format::context::common::StreamIter) -> Option<VideoStream> {
        let video_stream = stream_iter.best(ffmpeg::media::Type::Video);

        match video_stream {
            Some(video_stream) => {
                if video_stream.frames() > 0 {
                    Some(VideoStream {
                        index: video_stream.index(),
                        time_base: video_stream.time_base(),
                        start_time: video_stream.start_time(),
                        duration: video_stream.duration(),
                        frames: video_stream.frames(),
                        disposition: video_stream.disposition(),
                        rate: video_stream.rate(),
                        avg_frame_rate: video_stream.avg_frame_rate(),
                    })
                }
                else {
                    None
                }
            },
            None => None,
        }
    }
}


// TODO: use composition to factorize the members common to Video and Audio
pub struct AudioStream {
    pub index: usize,
    pub time_base: Rational,
    pub start_time: i64,
    pub duration: i64,
    pub frames: i64,
    pub disposition: Disposition,
    pub rate: Rational,
    pub avg_frame_rate: Rational,
}

impl AudioStream {
    pub fn new(stream_iter: &ffmpeg::format::context::common::StreamIter) -> Option<AudioStream> {
        let audio_stream = stream_iter.best(ffmpeg::media::Type::Audio);

        match audio_stream {
            Some(audio_stream) => {
                if audio_stream.duration() > 0 || audio_stream.frames() > 0 {
                    Some(AudioStream {
                        index: audio_stream.index(),
                        time_base: audio_stream.time_base(),
                        start_time: audio_stream.start_time(),
                        duration: audio_stream.duration(),
                        frames: audio_stream.frames(),
                        disposition: audio_stream.disposition(),
                        rate: audio_stream.rate(),
                        avg_frame_rate: audio_stream.avg_frame_rate(),
                    })
                }
                else {
                    None
                }
            },
            None => None,
        }
    }
}
