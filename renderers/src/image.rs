use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

// This is from https://github.com/gtk-rs/examples/blob/master/src/bin/cairo_threads.rs
// Helper struct that allows passing the pixels to the Cairo image surface and once the
// image surface is destroyed the pixels will be stored in the return_location.
//
// This allows us to give temporary ownership of the pixels to the Cairo surface and later
// retrieve them back in a safe way while ensuring that nothing else still has access to
// it.
struct ImageHolder {
    pixels: Option<Box<[u8]>>,
    return_location: Rc<RefCell<Option<Box<[u8]>>>>,
}

// This stores the pixels back into the return_location as now nothing
// references the pixels anymore
impl Drop for ImageHolder {
    fn drop(&mut self) {
        *self.return_location.borrow_mut() = Some(self.pixels.take().expect("Holding no image"));
    }
}

impl AsMut<[u8]> for ImageHolder {
    fn as_mut(&mut self) -> &mut [u8] {
        self.pixels.as_mut().expect("Holding no image").as_mut()
    }
}

// This is mostly from https://github.com/gtk-rs/examples/blob/master/src/bin/cairo_threads.rs
// This stores a heap allocated byte array for the pixels for each of our RGB24
// images, can be sent safely between threads and can be temporarily converted to a Cairo image
// surface for drawing operations
//
// Note that the alignments here a forced to Cairo's requirements
// Force dimensions to `u16` so that we are sure to fit in the `i32` dimensions
pub struct Image {
    // Use a Cell to hide internal implementation details due to Cairo surface ownership
    pixels: Cell<Option<Box<[u8]>>>,
    pub width: i32,
    pub height: i32,
    stride: i32,
}

impl Image {
    pub fn try_new(width: i32, height: i32) -> Result<Self, String> {
        assert!(width > 0);
        assert!(height > 0);

        let stride = cairo::Format::Rgb24
            .stride_for_width(width as u32)
            .map_err(|status| {
                format!("Couldn't compute stride for width {}: {:?}", width, status)
            })?;

        Ok(Image {
            pixels: Cell::new(Some(vec![0; height as usize * stride as usize].into())),
            width,
            height,
            stride,
        })
    }

    pub fn from_unknown(input: &[u8]) -> Result<Self, String> {
        let image = image::load_from_memory(input)
            .map_err(|err| format!("Error loading image: {:?}", err))?;

        match image.as_rgb8().as_ref() {
            Some(rgb_image) => {
                // Align to Cairo's needs: 4 bytes per pixel
                // When converting to RGB8, image crate uses 3 bytes in different order
                let width = rgb_image.width();
                let height = rgb_image.height();

                if width > i32::max_value() as u32 {
                    return Err(format!("Image width {} is too large", width));
                }
                if height > i32::max_value() as u32 {
                    return Err(format!("Image height {} is too large", height));
                }

                let stride = cairo::Format::Rgb24
                    .stride_for_width(width)
                    .map_err(|status| {
                        format!("Couldn't compute stride for width {}: {:?}", width, status)
                    })?;

                let width = width as i32;
                let height = height as i32;

                let mut pixels = Vec::with_capacity(height as usize * stride as usize);

                for pixel in rgb_image.chunks(3) {
                    pixels.push(pixel[2]);
                    pixels.push(pixel[1]);
                    pixels.push(pixel[0]);
                    pixels.push(0);
                }

                Ok(Image {
                    pixels: Cell::new(Some(pixels.into())),
                    width,
                    height,
                    stride,
                })
            }
            None => Err("Error converting image to raw RGB".to_owned()),
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    #[allow(dead_code)]
    pub fn stride(&self) -> i32 {
        self.stride
    }

    // Calls the given closure with a temporary Cairo image surface. After the closure has returned
    // there must be no further references to the surface.
    pub fn with_surface<F: FnOnce(&cairo::ImageSurface)>(&self, func: F) {
        // Temporary move out the pixels
        let pixels = self.pixels.take();
        assert!(pixels.is_some());

        // A new return location that is then passed to our helper struct below
        let return_location = Rc::new(RefCell::new(None));
        {
            let holder = ImageHolder {
                pixels,
                return_location: Rc::clone(&return_location),
            };

            // The surface will own the image for the scope of the block below
            {
                let surface = cairo::ImageSurface::create_for_data(
                    holder,
                    cairo::Format::Rgb24,
                    self.width,
                    self.height,
                    self.stride,
                )
                .expect("Can't create surface");
                func(&surface);
            }

            // Now the surface will be destroyed and the pixels are stored in the return_location
        }

        // Move the pixels back
        let pixels = return_location.borrow_mut().take();
        assert!(pixels.is_some());

        self.pixels.set(pixels);
    }

    // Calls the given closure with a temporary Cairo image surface. After the closure has returned
    // there must be no further references to the surface.
    pub fn with_surface_external_context<F: FnOnce(&cairo::Context, &cairo::ImageSurface)>(
        &self,
        cr: &cairo::Context,
        func: F,
    ) {
        // Temporary move out the pixels
        let pixels = self.pixels.take();
        assert!(pixels.is_some());

        // A new return location that is then passed to our helper struct below
        let return_location = Rc::new(RefCell::new(None));
        {
            let holder = ImageHolder {
                pixels,
                return_location: Rc::clone(&return_location),
            };

            // The surface will own the image for the scope of the block below
            {
                let surface = cairo::ImageSurface::create_for_data(
                    holder,
                    cairo::Format::Rgb24,
                    self.width,
                    self.height,
                    self.stride,
                )
                .expect("Can't create surface");
                func(cr, &surface);

                // Release the reference to the surface.
                // This is required otherwise the surface is not released and the pixels
                // are not moved back to `return_location`
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            }

            // Now the surface will be destroyed and the pixels are stored in the return_location
        }

        // Move the pixels back
        let pixels = return_location.borrow_mut().take();
        assert!(pixels.is_some());

        self.pixels.set(pixels);
    }
}
