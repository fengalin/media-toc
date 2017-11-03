extern crate image;

pub struct AlignedImage {
    aligned_buffer: Vec<u8>,
    width: usize,
    height: usize,
    stride: usize,
}

impl AlignedImage {
    pub fn from_uknown_buffer(input: &[u8]) -> Result<Self, String> {
        match image::load_from_memory(input) {
            Ok(image) => {
                match image.as_rgb8().as_ref() {
                    Some(rgb_image) => {
                        // Fix stride: there must be a better way...
                        // Align to Cairo's needs: 4 bytes per pixel
                        // When converting to RGB8, image crate uses 3 bytes in different order
                        let width = rgb_image.width() as usize;
                        let height = rgb_image.height() as usize;
                        let bytes_per_pixels = 4;
                        let stride = width * bytes_per_pixels;

                        let mut new_img = AlignedImage {
                            aligned_buffer: Vec::with_capacity(height * stride),
                            width: width,
                            height: height,
                            stride: stride,
                        };

                        for pixel in rgb_image.chunks(3) {
                            new_img.aligned_buffer.push(pixel[2]);
                            new_img.aligned_buffer.push(pixel[1]);
                            new_img.aligned_buffer.push(pixel[0]);
                            new_img.aligned_buffer.push(0);
                        }

                        Ok(new_img)
                    }
                    None => Err("Error converting image to raw RGB".to_owned()),
                }
            }
            Err(error) => Err(format!("Error loading image: {:?}", error)),
        }
    }

    pub fn into_boxed_slice(self) -> Box<[u8]> {
        self.aligned_buffer.into_boxed_slice()
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn stride(&self) -> usize {
        self.stride
    }
}
