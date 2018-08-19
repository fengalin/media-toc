use cairo;
use image;
use log::warn;

pub struct ImageSurface {}

impl ImageSurface {
    pub fn create_from_uknown(input: &[u8]) -> Result<cairo::ImageSurface, String> {
        match image::load_from_memory(input) {
            Ok(image) => {
                match image.as_rgb8().as_ref() {
                    Some(rgb_image) => {
                        // Align to Cairo's needs: 4 bytes per pixel
                        // When converting to RGB8, image crate uses 3 bytes in different order
                        let width = rgb_image.width() as i32;
                        let height = rgb_image.height() as i32;
                        let bytes_per_pixels = 4;
                        let stride = width * bytes_per_pixels;

                        let mut aligned_buffer = Vec::with_capacity((height * stride) as usize);

                        for pixel in rgb_image.chunks(3) {
                            aligned_buffer.push(pixel[2]);
                            aligned_buffer.push(pixel[1]);
                            aligned_buffer.push(pixel[0]);
                            aligned_buffer.push(0);
                        }

                        cairo::ImageSurface::create_for_data(
                            aligned_buffer.into_boxed_slice(),
                            cairo::Format::Rgb24,
                            width,
                            height,
                            stride,
                        ).map_err(|err| {
                            let msg = format!(
                                "Error creating ImageSurface from aligned image: {:?}",
                                err,
                            );
                            warn!("{}", msg);
                            msg
                        })
                    }
                    None => {
                        let msg = "Error converting image to raw RGB".to_owned();
                        warn!("{}", msg);
                        Err(msg)
                    }
                }
            }
            Err(err) => {
                let msg = format!("Error loading image: {:?}", err);
                warn!("{}", msg);
                Err(msg)
            }
        }
    }
}
