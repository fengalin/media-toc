extern crate cairo;

#[cfg(feature = "profiling-waveform-image")]
use chrono::Utc;

use media::{AudioBuffer, AudioBufferIter, SAMPLES_NORM};

pub const BACKGROUND_COLOR: (f64, f64, f64) = (0.2f64, 0.2235f64, 0.2314f64);

// initial image dimensions
// will dynamically adapt if needed
const INIT_WIDTH: i32 = 2000;
const INIT_HEIGHT: i32 = 450;

pub struct WaveformImage {
    is_ready: bool,
    exposed_image: Option<cairo::ImageSurface>,
    working_image: Option<cairo::ImageSurface>,
    image_offset: Option<f64>,

    req_width: i32,
    req_height: i32,
    req_step_duration: u64,

    pub first_sample: usize,
    pub last_sample: usize,
    pub sample_window: usize,
    pub sample_step: usize,
    pub sample_step_f: f64,
}

impl WaveformImage {
    pub fn new() -> Self {
        WaveformImage {
            is_ready: false,
            exposed_image: Some(
                cairo::ImageSurface::create(
                    cairo::Format::Rgb24, INIT_WIDTH, INIT_HEIGHT
                ).unwrap()
            ),
            working_image: Some(
                cairo::ImageSurface::create(
                    cairo::Format::Rgb24, INIT_WIDTH, INIT_HEIGHT
                ).unwrap()
            ),
            image_offset: None,

            req_width: 0,
            req_height: 0,
            req_step_duration: 0,

            first_sample: 0,
            last_sample: 0,
            sample_window: 0,
            sample_step: 0,
            sample_step_f: 0f64,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        self.is_ready = false;

        // self.exposed_image & self.working_image
        // will be cleaned on next with draw

        self.image_offset = None;

        self.req_width = 0;
        self.req_height = 0;
        self.req_step_duration = 0;

        self.first_sample = 0;
        self.last_sample = 0;
        self.sample_window = 0;
        self.sample_step = 0;
        self.sample_step_f = 0f64;
    }

    pub fn update_dimensions(&mut self, duration: u64, width: i32, height: i32) {
        self.req_width = width;
        self.req_height = height;

        // resolution
        let width = width as u64;
        self.req_step_duration =
            if duration > width {
                duration / width
            } else {
                1
            };
    }

    pub fn is_ready(&self) -> bool {
        self.is_ready
    }

    pub fn get_image(&self) -> &cairo::ImageSurface {
        self.exposed_image.as_ref().unwrap()
    }

    pub fn update_from_other(&mut self, other: &WaveformImage) {
        self.sample_step = other.sample_step;
        self.sample_step_f = other.sample_step_f;
        self.req_step_duration = other.req_step_duration;
    }

    // Render the waveform within the provided limits.
    // This function is called from a working buffer
    // which means that self.exposed_image is the image
    // that was previously exposed to the UI.
    // This also means that we can safely deal with both
    // images since none of them is exposed at this very moment.
    // The rendering process reuses the previously rendered image
    // whenever possible.
    pub fn render(&mut self,
        audio_buffer: &AudioBuffer,
        first_sample: usize,
        last_sample: usize,
        sample_duration: f64,
    ) {
        #[cfg(feature = "profiling-waveform-image")]
        let start = Utc::now();

        // use an integer number of samples per step
        let sample_step_f = self.req_step_duration as f64 / sample_duration;
        let sample_step = sample_step_f as usize;

        if audio_buffer.samples.len() < sample_step {
            // buffer too small to render
            return;
        }

        // Align requested first and last samples in order to keep a steady
        // offset between redraws. This allows using the same samples
        // for a given req_step_duration and avoiding flickering
        // between redraws
        let mut first_sample =
            first_sample / sample_step * sample_step;
        if first_sample < audio_buffer.first_sample {
            // first sample might be smaller than audio_buffer.first_sample
            // due to alignement on sample_step
            first_sample += sample_step;
        }

        let last_sample = last_sample / sample_step * sample_step;

        let extraction_samples_window = (last_sample - first_sample) / sample_step;

        let mut must_redraw = self.exposed_image.is_none() || self.sample_step != sample_step;
        if !must_redraw && first_sample >= self.first_sample
        && last_sample <= self.last_sample
        {   // traget extraction fits in previous extraction
            return;
        } else if first_sample + extraction_samples_window < self.first_sample
            || first_sample > self.last_sample
        {   // current samples extraction doesn't overlap with samples in previous image
            must_redraw = true;
        }

        let mut y_offset = 0f64;
        let (working_image, previous_image) = {
            let mut can_reuse = false;
            let target_width = (extraction_samples_window as i32).max(self.req_width);

            if let Some(ref working_image) = self.working_image {
                if self.req_height != working_image.get_height() {
                    // height has changed => scale samples amplitude accordingly
                    must_redraw = true;
                }

                if target_width <= working_image.get_width()
                && self.req_height <= working_image.get_height() {
                    // expected dimensions fit in current working image => reuse it
                    can_reuse = true;
                    if self.req_height != working_image.get_height() {
                        y_offset = (self.req_height - working_image.get_height()) as f64;
                    }
                }
            }

            if can_reuse {
                (
                    self.working_image.take().unwrap(),
                    self.exposed_image.take().unwrap(),
                )
            } else {
                // can't reuse => create new images and force redraw
                must_redraw = true;

                (
                    cairo::ImageSurface::create( // working_image
                        cairo::Format::Rgb24,
                        target_width,
                        self.req_height
                    ).expect(
                        &format!(
                            "WaveformBuffer.render: couldn't create image surface with width {}",
                            target_width,
                        )
                    ),
                    cairo::ImageSurface::create( // will be used as previous_image
                        cairo::Format::Rgb24,
                        target_width,
                        self.req_height
                    ).unwrap() // working_image could be created with same dimensions
                )
            }
        };

        let cr = cairo::Context::new(&working_image);

        if must_redraw {
            // Initialization or resolution has changed or seek requested
            // redraw the whole range
            self.image_offset = None;
            self.draw(&cr,
                audio_buffer.iter(first_sample, last_sample, sample_step),
                0f64,
                working_image.get_width()
            );

            self.sample_step = sample_step;
            self.sample_step_f = sample_step_f;
            self.first_sample = first_sample;
            self.last_sample = last_sample;
        } else {
            // can reuse previous context
            // Note: condition first_sample >= self.self.first_sample
            //                 && last_sample <= self.self.last_sample
            // (traget extraction fits in previous extraction)
            // already checked

            if first_sample < self.first_sample {
                // append samples before previous first sample
                let image_width_as_samples =
                    working_image.get_width() as usize * sample_step;

                self.last_sample = self.last_sample.min(
                    first_sample + image_width_as_samples
                );

                // shift previous image to the right
                let x_offset = (
                    (self.first_sample - first_sample) / sample_step
                ) as f64;

                println!(
                    "WaveformImage: appending {} samples before previous first sample",
                    self.first_sample - first_sample
                );

                self.copy_image(&cr, &previous_image, x_offset, y_offset);
                self.draw(&cr,
                    audio_buffer.iter(first_sample, self.first_sample, sample_step),
                    0f64,
                    working_image.get_width()
                );

                self.first_sample = first_sample;
            } else {
                // first_sample >= self.first_sample
                // Note: due to previous conditions tested before,
                // this also implies:
                assert!(last_sample > self.last_sample);

                // shift previous image to the left (if necessary)
                let x_offset = (
                    (first_sample - self.first_sample) / sample_step
                ) as f64;
                self.copy_image(&cr, &previous_image, -x_offset, y_offset);

                // append samples after previous last sample
                let first_sample_to_draw = self.last_sample.max(first_sample);
                let first_x_to_draw = (
                    (first_sample_to_draw - self.first_sample) / sample_step
                ) as f64 - x_offset;

                self.draw(&cr,
                    audio_buffer.iter(first_sample_to_draw, last_sample, sample_step),
                    first_x_to_draw,
                    working_image.get_width()
                );

                self.first_sample = first_sample;
                self.last_sample = last_sample;
            }
        }

        // swap images
        self.working_image = Some(previous_image);
        self.exposed_image = Some(working_image);

        self.sample_window = self.last_sample - self.first_sample;
        self.is_ready = true;

        #[cfg(feature = "profiling-waveform-image")]
        let end = Utc::now();

        #[cfg(feature = "profiling-waveform-image")]
        println!("waveform-image,{},{}",
            start.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
        );
    }

    fn copy_image(&mut self,
        cr: &cairo::Context,
        previous_image: &cairo::ImageSurface,
        image_offset: f64,
        y_offset: f64
    ) {
        self.image_offset = Some(image_offset);
        cr.set_source_surface(&previous_image, image_offset, y_offset);
        cr.paint();
    }

    fn draw(&self,
        cr: &cairo::Context,
        mut sample_iter: AudioBufferIter,
        first_x: f64,
        width: i32,
    ) {
        // clear what needs to be cleaned
        {
            let scale_y = f64::from(self.req_height) / SAMPLES_NORM;

            match self.image_offset {
                Some(image_offset) => { // image results from a copy a a previous image
                    cr.scale(1f64, scale_y);

                    if image_offset <= 0f64 && first_x > 0f64 {
                        // image was shifted left => clear samples on the right
                        self.clear_area(&cr, first_x, f64::from(width));
                    } else {
                        // image was shifted right => clear samples on the left
                        self.clear_area(&cr, first_x, image_offset);
                    }
                },
                None => { // draw image completely => set background
                    self.clear_image(&cr);
                    cr.scale(1f64, scale_y);
                },
            }
        }

        // render the requested samples
        if sample_iter.size_hint().0 > 0 {
            // Stroke selected samples
            cr.set_line_width(0.5f64);
            cr.set_source_rgb(0.8f64, 0.8f64, 0.8f64);

            let mut x = first_x;
            let mut sample_value = *sample_iter.next().unwrap();
            for sample in sample_iter {
                cr.move_to(x, sample_value);
                x += 1f64;
                sample_value = *sample;
                cr.line_to(x, sample_value);
                cr.stroke();
            }
        }
    }

     fn clear_image(&self, cr: &cairo::Context) {
        cr.set_source_rgb(
            BACKGROUND_COLOR.0,
            BACKGROUND_COLOR.1,
            BACKGROUND_COLOR.2
        );
        cr.paint();
    }

    // clear samples previously rendered
    fn clear_area(&self, cr: &cairo::Context, first_x: f64, limit_x: f64) {
        cr.set_source_rgb(
            BACKGROUND_COLOR.0,
            BACKGROUND_COLOR.1,
            BACKGROUND_COLOR.2
        );
        cr.rectangle(first_x, 0f64, limit_x - first_x, SAMPLES_NORM);
        cr.fill();
    }
}
