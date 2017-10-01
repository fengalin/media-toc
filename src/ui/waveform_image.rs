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
    pub is_ready: bool,
    exposed_image: Option<cairo::ImageSurface>,
    working_image: Option<cairo::ImageSurface>,

    req_width: i32,
    req_height: i32,
    req_step_duration: u64,
    force_redraw: bool,

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

            req_width: 0,
            req_height: 0,
            req_step_duration: 0,
            force_redraw: false,

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

        self.req_width = 0;
        self.req_height = 0;
        self.req_step_duration = 0;
        self.force_redraw = false;

        self.first_sample = 0;
        self.last_sample = 0;
        self.sample_window = 0;
        self.sample_step = 0;
        self.sample_step_f = 0f64;
    }

    pub fn update_dimensions(&mut self, duration: u64, width: i32, height: i32) {
        // if the requested height is different from current height
        // it might be necessary to force rendering when stream
        // is paused or eos

        let width_f = width as u64;
        let req_step_duration =
            if duration > width_f {
                duration / width_f
            } else {
                1
            };

        self.force_redraw =
            self.req_height != height
            || self.req_width != width
            || self.req_step_duration != req_step_duration;

        self.req_width = width;
        self.req_height = height;
        self.req_step_duration = req_step_duration;
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
        self.req_width = other.req_width;
        self.req_height = other.req_height;
        self.force_redraw = other.force_redraw;
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
        let sample_step = (sample_step_f as usize).max(1);

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

        let mut must_redraw = !self.is_ready || self.force_redraw;
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

            let working_image = self.working_image.take().unwrap();
            if self.req_height != working_image.get_height() {
                // height has changed => scale samples amplitude accordingly
                must_redraw = true;
                y_offset = (self.req_height - working_image.get_height()) as f64;
            }

            if target_width <= working_image.get_width()
            && self.req_height <= working_image.get_height() {
                // expected dimensions fit in current working image => reuse it
                can_reuse = true;
            }

            if can_reuse {
                (
                    working_image,
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
            self.redraw(&cr,
                audio_buffer.iter(first_sample, last_sample, sample_step),
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
                let sample_offset = self.first_sample - first_sample;
                let x_offset = sample_offset as f64 / self.sample_step_f;

                println!(
                    "WaveformImage: appending {} samples before previous first sample",
                    self.first_sample - first_sample
                );

                let prev_first_sample = self.first_sample;
                self.append_left(&cr,
                    &previous_image,
                    x_offset,
                    y_offset,
                    audio_buffer.iter(first_sample, prev_first_sample, sample_step),
                );

                self.first_sample = first_sample;
                self.last_sample = self.last_sample + sample_offset;
            }

            if last_sample > self.last_sample {
                // shift previous image to the left (if necessary)
                // and append missing samples to the right
                let x_offset = -((
                    (first_sample - self.first_sample) / self.sample_step
                ) as f64);

                // append samples after previous last sample
                let first_sample_to_draw = self.last_sample.max(first_sample);
                let first_x_to_draw = (
                    (first_sample_to_draw - self.first_sample) / sample_step
                ) as f64 + x_offset;

                self.append_right(&cr,
                    &previous_image,
                    x_offset,
                    y_offset,
                    audio_buffer.iter(first_sample_to_draw, last_sample, sample_step),
                    first_x_to_draw,
                    working_image.get_width() // clear_limit
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
        self.force_redraw = false;

        #[cfg(feature = "profiling-waveform-image")]
        let end = Utc::now();

        #[cfg(feature = "profiling-waveform-image")]
        println!("waveform-image,{},{}",
            start.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
        );
    }

    // redraw the whole sample range on a clean image
    fn redraw(&mut self,
        cr: &cairo::Context,
        sample_iter: AudioBufferIter
    ) {
        cr.set_source_rgb(
            BACKGROUND_COLOR.0,
            BACKGROUND_COLOR.1,
            BACKGROUND_COLOR.2
        );
        cr.paint();

        self.set_scale(&cr);
        self.draw_samples(cr, sample_iter, 0f64);
    }

    fn append_left(&mut self,
        cr: &cairo::Context,
        previous_image: &cairo::ImageSurface,
        x_offset: f64,
        y_offset: f64,
        sample_iter: AudioBufferIter,
    ) {
        cr.set_source_surface(previous_image, x_offset, y_offset);
        cr.paint();

        self.set_scale(&cr);
        self.clear_area(&cr, 0f64, x_offset);
        self.draw_samples(cr, sample_iter, 0f64);
    }

    fn append_right(&mut self,
        cr: &cairo::Context,
        previous_image: &cairo::ImageSurface,
        x_offset: f64,
        y_offset: f64,
        sample_iter: AudioBufferIter,
        first_x: f64,
        clear_limit: i32,
    ) {
        cr.set_source_surface(previous_image, x_offset, y_offset);
        cr.paint();

        self.set_scale(&cr);
        self.clear_area(&cr, first_x, f64::from(clear_limit));
        self.draw_samples(cr, sample_iter, first_x);
    }

    fn set_scale(&self, cr: &cairo::Context) {
        cr.scale(1f64, f64::from(self.req_height) / SAMPLES_NORM);
    }

    fn draw_samples(&self,
        cr: &cairo::Context,
        mut sample_iter: AudioBufferIter,
        first_x: f64,
    ) {
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

#[cfg(test)]
mod tests {
    extern crate cairo;

    extern crate gstreamer as gst;
    extern crate gstreamer_audio as gst_audio;

    use std::fs::{create_dir, File};
    use std::io::ErrorKind;

    use std::{i16, u16};

    use media::AudioBuffer;
    use ui::WaveformImage;

    const OUT_DIR: &'static str = "target/test";
    const SAMPLE_RATE: i32 = 300;

    fn prepare_tests() {
        match create_dir(&OUT_DIR) {
            Ok(_) => (),
            Err(error) => match error.kind() {
                ErrorKind::AlreadyExists => (),
                _ =>
                    panic!("WaveformImage test: couldn't create directory {}",
                        OUT_DIR
                    ),
            },
        }
    }

    #[test]
    fn multiple_renderings() {
        gst::init().unwrap();

        prepare_tests();

        // AudioBuffer
        let mut audio_buffer = AudioBuffer::new(1_000_000_000); // 1s
        let caps = gst::Caps::new_simple(
            "audio/x-raw",
            &[
                ("format", &gst_audio::AUDIO_FORMAT_S16.to_string()),
                ("layout", &"interleaved"),
                ("channels", &1),
                ("rate", &SAMPLE_RATE),
            ],
        );
        audio_buffer.set_caps(&caps);

        // WaveformImage
        let mut waveform = WaveformImage::new();
        waveform.update_dimensions(
            1_000_000_000, // 1s
            SAMPLE_RATE,
            SAMPLE_RATE
        );

        // Compute a buffer in the specified range
        // which will be rendered as a diagonal on the Waveform image
        // from left top corner to right bottom of the target image
        // if all samples are rendered in the range [0:SAMPLE_RATE]
        fn build_buffer(first_sample: usize, last_sample: usize) -> Vec<i16> {
            let mut buffer: Vec<i16> = Vec::new();
            let mut index = first_sample;
            while index < last_sample {
                buffer.push((
                    i16::MAX as i32
                    - (index as f64 / SAMPLE_RATE as f64 * u16::MAX as f64
                    ) as i32
                ) as i16);
                index += 1;
            }
            buffer
        }

        fn render_with_samples(
            waveform: &mut WaveformImage,
            audio_buffer: &mut AudioBuffer,
            caps: &gst::Caps,
            first: usize,
            last: usize,
            segement_first_sample: usize,
        ) {
            audio_buffer.push_samples(
                &build_buffer(first, last),
                first,
                segement_first_sample,
                &caps
            );
            let first_sample_to_extract =
                if audio_buffer.last_sample
                    > audio_buffer.first_sample + SAMPLE_RATE as usize
                {
                    audio_buffer.last_sample - SAMPLE_RATE as usize
                } else {
                    audio_buffer.first_sample
                };

            waveform.render(&audio_buffer,
                first_sample_to_extract,
                audio_buffer.last_sample,
                audio_buffer.sample_duration,
            );

            let image = waveform.get_image();

            let mut output_file = File::create(
                    format!(
                        "{}/waveform_image_{:03}_{:03}.png", OUT_DIR,
                        waveform.first_sample,
                        waveform.last_sample
                    )
                ).expect("WaveformImage test: couldn't create output file");
            image.write_to_png(&mut output_file)
                .expect("WaveformImage test: couldn't write waveform image");
        }

        render_with_samples(&mut waveform, &mut audio_buffer, &caps, 100, 200, 100);
        // overlap on the left
        render_with_samples(&mut waveform, &mut audio_buffer, &caps,  50, 150, 50);
        render_with_samples(&mut waveform, &mut audio_buffer, &caps,   0, 100, 0);
        // appended to the right
        render_with_samples(&mut waveform, &mut audio_buffer, &caps, 200, 300, 200);

        // scrolling and overlaping on the right
        render_with_samples(&mut waveform, &mut audio_buffer, &caps, 250, 350, 250);
    }
}
