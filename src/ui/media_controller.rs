extern crate gtk;
extern crate ffmpeg;

use gtk::WidgetExt;

use ::media::Context;

pub struct MediaController {
    container: gtk::Grid,
    stream_index: Option<usize>,
}

impl MediaController {
    pub fn new(container: gtk::Grid) -> MediaController {
        MediaController{ container: container, stream_index: None }
    }

    // FIXME: are there any annotations for setters/getters?
    pub fn set_index(&mut self, index: usize) {
        self.stream_index = Some(index);
    }

    pub fn stream_index(&self) -> usize {
        self.stream_index.unwrap()
    }

    pub fn show(&self) {
        self.container.show();
    }

    pub fn hide(&self) {
        self.container.hide();
    }
}

pub trait NotifiableMedia {
    fn new_media(&mut self, &mut Context);

    fn new_packet(&mut self, stream: &ffmpeg::format::stream::Stream, packet: &ffmpeg::codec::packet::Packet) {
        println!("stream: {}", stream.index());
        println!("packet: size: {} - duration: {}", packet.size(), packet.duration());
        if let Some(data) = packet.data() {
            println!("found data");
        }

        let data_iter = stream.side_data();
        println!("side data nb: {}", data_iter.size_hint().0);
    }
}
