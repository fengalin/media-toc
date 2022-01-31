use gst::{glib, prelude::*};

pub(crate) mod bin;

glib::wrapper! {
    pub struct RendererBin(ObjectSubclass<bin::RendererBin>) @extends gst::Bin, gst::Element, gst::Object;
}

impl RendererBin {
    /// Asynchronoulsy call the provided function on the parent.
    ///
    /// This is useful to set the state of the pipeline.
    fn parent_call_async(&self, f: impl FnOnce(&RendererBin, &gst::Element) + Send + 'static) {
        match self.parent() {
            Some(parent) => parent.downcast_ref::<gst::Element>().unwrap().call_async(
                glib::clone!(@weak self as bin => @default-panic, move |parent| f(&bin, parent)),
            ),
            None => unreachable!("media-toc renderer bin has no parent"),
        }
    }
}

unsafe impl Send for RendererBin {}
unsafe impl Sync for RendererBin {}

pub use bin::NAME as RENDERER_BIN_NAME;

glib::wrapper! {
    pub struct Renderer(ObjectSubclass<renderer::Renderer>) @extends gst::Element, gst::Object;
}

unsafe impl Send for Renderer {}
unsafe impl Sync for Renderer {}

pub(crate) mod renderer;
pub use renderer::{
    SeekField, SegmentField, BUFFER_SIZE_PROP, CLOCK_REF_PROP, DBL_RENDERER_IMPL_PROP,
    MUST_REFRESH_SIGNAL, NAME as RENDERER_NAME, SEGMENT_DONE_SIGNAL,
};

pub(crate) use renderer::GET_WINDOW_TIMESTAMPS_SIGNAL;

pub const PLAY_RANGE_DONE_SIGNAL: &str = "play-range-done";

gst::plugin_define!(
    mediatocvisu,
    env!("CARGO_PKG_DESCRIPTION"),
    plugin_init,
    concat!(env!("CARGO_PKG_VERSION"), "-", env!("COMMIT_ID")),
    "MIT/X11",
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_REPOSITORY"),
    env!("BUILD_REL_DATE")
);

pub fn init() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        gst::init().unwrap();
        self::plugin_register_static().expect("media-toc rendering plugin init");
    });
}

fn plugin_init(plugin: &gst::Plugin) -> Result<(), glib::BoolError> {
    gst::Element::register(
        Some(plugin),
        bin::NAME,
        gst::Rank::None,
        RendererBin::static_type(),
    )?;
    gst::Element::register(
        Some(plugin),
        renderer::NAME,
        gst::Rank::None,
        Renderer::static_type(),
    )?;
    Ok(())
}
