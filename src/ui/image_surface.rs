extern crate cairo;

use ::media::AlignedImage;

pub struct ImageSurface {
    pub surface: cairo::ImageSurface,
}

impl ImageSurface {
    pub fn from_aligned_image(image: AlignedImage) -> Result<Self, String> {
        let width = image.width() as i32;
        let height = image.height() as i32;
        let stride = image.stride() as i32;

        Ok(ImageSurface {
            surface: cairo::ImageSurface::create_for_data(
                image.into_boxed_slice(),
                |_| {}, cairo::Format::Rgb24,
                width, height, stride
            ),
        })
    }
}

