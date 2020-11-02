use gst::{prelude::*, TagSetterExt, TocSetterExt};

use super::{Exporter, MediaInfo};

static EXTENSION: &str = "toc.mkv";
static AUDIO_EXTENSION: &str = "toc.mka";

#[derive(Default)]
pub struct MatroskaTocFormat;

impl MatroskaTocFormat {
    pub fn new() -> Self {
        MatroskaTocFormat
    }

    pub fn extension() -> &'static str {
        EXTENSION
    }

    pub fn audio_extension() -> &'static str {
        AUDIO_EXTENSION
    }
}

impl Exporter for MatroskaTocFormat {
    fn export(&self, info: &MediaInfo, destination: &gst::Element) {
        {
            let tag_setter = destination
                .clone()
                .dynamic_cast::<gst::TagSetter>()
                .expect("MatroskaTocFormat::export muxer is not a TagSetter");

            tag_setter.merge_tags(&info.fixed_tags(), gst::TagMergeMode::Replace)
        }

        if let Some(ref toc) = info.toc {
            let toc_setter = destination
                .clone()
                .dynamic_cast::<gst::TocSetter>()
                .expect("MatroskaTocFormat::export muxer is not a TocSetter");

            toc_setter.set_toc(Some(toc));
        }
    }
}
