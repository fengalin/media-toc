pub mod dbl_renderer;
pub use dbl_renderer::{DoubleRenderer, DoubleRendererImpl, GBoxedDoubleRendererImpl};

pub mod renderer;
pub use renderer::Renderer;

pub mod prelude {
    pub use super::{DoubleRendererImpl, Renderer};
}
