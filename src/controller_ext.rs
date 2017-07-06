extern crate ffmpeg;


pub trait Notifiable {
    fn notify_new_media(&mut self, stream: Option<ffmpeg::Stream>);
}
