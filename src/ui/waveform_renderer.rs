extern crate cairo;

use media::{AudioBufferIter, SAMPLES_NORM};

pub const BACKGROUND_COLOR: (f64, f64, f64) = (0.2f64, 0.2235f64, 0.2314f64);

pub struct WaveformRenderer {
    cr: cairo::Context,
    width: f64,
    scale_y: f64,
    image_offset: Option<f64>,
}

impl WaveformRenderer {
    pub fn new(image: &cairo::ImageSurface) -> Self {
        WaveformRenderer {
            cr: cairo::Context::new(&image),
            width: f64::from(image.get_width()),
            scale_y: f64::from(image.get_height()) / SAMPLES_NORM,
            image_offset: None,
        }
    }

    pub fn copy_image(&mut self, previous_image: &cairo::ImageSurface, image_offset: f64) {
        self.image_offset = Some(image_offset);
        self.cr.set_source_surface(&previous_image, image_offset, 0f64);
        self.cr.paint();
    }

    pub fn draw_waveform(&self,
        mut sample_iter: AudioBufferIter,
        first_x: f64
    ) {
        // clear what's need to be cleanst
        match self.image_offset {
            Some(image_offset) => { // image results from a copy a a previous image
                self.cr.scale(1f64, self.scale_y);

                if image_offset <= 0f64 && first_x > 0f64 {
                    // image was shifted left => clear samples on the right
                    self.clear_area(first_x, self.width);
                } else {
                    // image was shifted right => clear samples on the left
                    self.clear_area(first_x, image_offset);
                }
            },
            None => { // image completely draw => set background
                self.clear_image();
                self.cr.scale(1f64, self.scale_y);
            },
        }

        // render the requested samples
        if sample_iter.size_hint().0 > 0 {
            // Stroke selected samples
            self.cr.set_line_width(0.5f64);
            self.cr.set_source_rgb(0.8f64, 0.8f64, 0.8f64);

            let mut x = first_x;
            let mut sample_value = *sample_iter.next().unwrap();
            for sample in sample_iter {
                self.cr.move_to(x, sample_value);
                x += 1f64;
                sample_value = *sample;
                self.cr.line_to(x, sample_value);
                self.cr.stroke();
            }
        }
    }

     fn clear_image(&self) {
        self.cr.set_source_rgb(
            BACKGROUND_COLOR.0,
            BACKGROUND_COLOR.1,
            BACKGROUND_COLOR.2
        );
        self.cr.paint();
    }

    // clear samples previously rendered
    fn clear_area(&self, first_x: f64, limit_x: f64) {
        self.cr.set_source_rgb(
            BACKGROUND_COLOR.0,
            BACKGROUND_COLOR.1,
            BACKGROUND_COLOR.2
        );
        self.cr.rectangle(first_x, 0f64, limit_x - first_x, SAMPLES_NORM);
        self.cr.fill();
    }
}
