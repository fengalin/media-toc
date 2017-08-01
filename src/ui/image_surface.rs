extern crate cairo;

use ::media::AlignedImage;

pub struct ImageSurface {
    pub surface: cairo::ImageSurface,
}

impl<'a> ImageSurface {
    pub fn from_uknown_buffer(input: &[u8]) -> Result<Self, String> {
        let aligned_image = match AlignedImage::from_uknown_buffer(input) {
            Ok(aligned_image) => aligned_image,
            Err(error) => return Err(error),
        };

        let width = aligned_image.width() as i32;
        let height = aligned_image.height() as i32;
        let stride = aligned_image.stride() as i32;

        Ok(ImageSurface {
            surface: cairo::ImageSurface::create_for_data(
                aligned_image.into_boxed_slice(),
                |_| {}, cairo::Format::Rgb24,
                width, height, stride
            ),
        })
    }
}

