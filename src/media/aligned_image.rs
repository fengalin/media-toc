extern crate image;

pub struct AlignedImage {
    aligned_buffer: Vec<u8>,
    width: usize,
    height: usize,
    stride: usize,
    bytes_per_pixels: usize,
    align_bytes: usize,
}

impl AlignedImage {
    pub fn from_uknown_buffer(input: &[u8]) -> Result<Self, String> {
        match image::load_from_memory(input) {
            Ok(image) => match image.as_rgb8().as_ref() {
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
                        bytes_per_pixels: bytes_per_pixels,
                        align_bytes: 1,
                    };

                    let mut pixel = [0; 4];
                    let mut byte_iter = rgb_image.iter();
                    loop {
                        match byte_iter.next() {
                            Some(byte) => pixel[2] = byte.clone(),
                            None => break,
                        };
                        let byte = byte_iter.next().unwrap();
                        pixel[1] = byte.clone();
                        let byte = byte_iter.next().unwrap();
                        pixel[0] = byte.clone();
                        new_img.aligned_buffer.extend_from_slice(&pixel);
                    }

                    Ok(new_img)
                },
                None => Err("Error converting image to raw RGB".to_owned()),
            },
            Err(error) => Err(format!("Error loading image: {:?}", error)),
        }
    }

    pub fn into_boxed_slice(self) -> Box<[u8]> {
        self.aligned_buffer.into_boxed_slice()
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.aligned_buffer.clone()
    }

    pub fn as_ref<'a>(&'a self) -> &'a Vec<u8> {
        self.aligned_buffer.as_ref()
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
