use glib::Cast;

use gstreamer as gst;
use gstreamer::{TagSetterExt, TocSetterExt};

use super::{Exporter, MediaInfo};

static EXTENSION: &'static str = "toc.mkv";
static AUDIO_EXTENSION: &'static str = "toc.mka";

pub struct MatroskaTocFormat {}

impl MatroskaTocFormat {
    pub fn get_extension() -> &'static str {
        EXTENSION
    }

    pub fn get_audio_extension() -> &'static str {
        AUDIO_EXTENSION
    }

    pub fn new() -> Self {
        MatroskaTocFormat {}
    }
}

impl Exporter for MatroskaTocFormat {
    fn export(&self, info: &MediaInfo, destination: &gst::Element) {
        {
            let tag_setter = destination
                .clone()
                .dynamic_cast::<gst::TagSetter>()
                .expect("MatroskaTocFormat::export muxer is not a TagSetter");

            tag_setter.merge_tags(&info.tags, gst::TagMergeMode::Replace)
        }

        if let Some(ref toc) = info.toc {
            let toc_setter = destination
                .clone()
                .dynamic_cast::<gst::TocSetter>()
                .expect("MatroskaTocFormat::export muxer is not a TocSetter");

            toc_setter.set_toc(toc);
        }
    }
}
