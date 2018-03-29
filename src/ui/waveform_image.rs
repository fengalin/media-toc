use cairo;

use std::i16;

#[cfg(feature = "dump-waveform")]
use chrono::Utc;

#[cfg(feature = "dump-waveform")]
use std::fs::{create_dir, File};
#[cfg(feature = "dump-waveform")]
use std::io::ErrorKind;

use media::{AudioBuffer, AudioChannel, AudioChannelSide};

pub const BACKGROUND_COLOR: (f64, f64, f64) = (0.2f64, 0.2235f64, 0.2314f64);
pub const AMPLITUDE_0_COLOR: (f64, f64, f64) = (0.5f64, 0.5f64, 0f64);

// Initial image dimensions
// will dynamically adapt if needed
const INIT_WIDTH: i32 = 2000;
const INIT_HEIGHT: i32 = 500;

// Samples normalization factores
lazy_static! {
    static ref SAMPLES_RANGE: f64 = f64::from(INIT_HEIGHT);
    static ref SAMPLES_OFFSET: f64 = *SAMPLES_RANGE / 2f64;
    static ref SAMPLES_SCALE_FACTOR: f64 = *SAMPLES_OFFSET / f64::from(i16::MAX);
}

#[cfg(feature = "dump-waveform")]
const WAVEFORM_DUMP_DIR: &str = "target/waveforms";

#[derive(Debug, Clone)]
pub struct WaveformSample {
    pub x: f64,
    pub values: Vec<f64>,
}

pub struct WaveformImage {
    pub id: usize,
    pub is_ready: bool,
    pub shareable_state_changed: bool,

    channel_colors: Vec<(f64, f64, f64)>,

    exposed_image: Option<cairo::ImageSurface>,
    secondary_image: Option<cairo::ImageSurface>,
    image_width: i32,
    image_width_f: f64,
    image_height: i32,
    image_height_f: f64,

    req_width: i32,
    req_height: i32,
    force_redraw: bool,

    pub lower: usize,
    pub upper: usize,

    pub contains_eos: bool,

    first: Option<WaveformSample>,
    pub last: Option<WaveformSample>,

    pub sample_step_f: f64,
    pub sample_step: usize,
    x_step_f: f64,
    pub x_step: usize,
}

impl WaveformImage {
    pub fn new(id: usize) -> Self {
        #[cfg(feature = "dump-waveform")]
        match create_dir(&WAVEFORM_DUMP_DIR) {
            Ok(_) => (),
            Err(error) => match error.kind() {
                ErrorKind::AlreadyExists => (),
                _ => panic!(
                    "WaveformImage::new couldn't create directory {}",
                    WAVEFORM_DUMP_DIR
                ),
            },
        }

        WaveformImage {
            id: id,
            is_ready: false,
            shareable_state_changed: false,

            channel_colors: Vec::new(),

            exposed_image: Some(
                cairo::ImageSurface::create(cairo::Format::Rgb24, INIT_WIDTH, INIT_HEIGHT).unwrap(),
            ),
            secondary_image: Some(
                cairo::ImageSurface::create(cairo::Format::Rgb24, INIT_WIDTH, INIT_HEIGHT).unwrap(),
            ),
            image_width: 0,
            image_width_f: 0f64,
            image_height: 0,
            image_height_f: 0f64,

            req_width: 0,
            req_height: 0,
            force_redraw: false,

            lower: 0,
            upper: 0,

            contains_eos: false,

            first: None,
            last: None,

            sample_step_f: 0f64,
            sample_step: 0,
            x_step_f: 0f64,
            x_step: 0,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        debug!("{}_cleanup", self.id);

        // self.exposed_image & self.secondary_image
        // will be cleaned on next with draw
        self.image_width = 0;
        self.image_width_f = 0f64;
        self.image_height = 0;
        self.image_height_f = 0f64;

        self.req_width = 0;
        self.req_height = 0;

        self.cleanup_sample_conditions();
    }

    pub fn cleanup_sample_conditions(&mut self) {
        debug!("{}_cleanup_sample_conditions", self.id);
        self.is_ready = false;
        self.force_redraw = false;

        self.channel_colors.clear();

        self.lower = 0;
        self.upper = 0;

        self.contains_eos = false;

        self.first = None;
        self.last = None;

        self.sample_step_f = 0f64;
        self.sample_step = 0;
        self.x_step_f = 0f64;
        self.x_step = 0;
    }

    pub fn set_channels(&mut self, channels: &[AudioChannel]) {
        debug!("{}_set_channels {}", self.id, channels.len());

        for channel in channels {
            self.channel_colors.push(match channel.side {
                AudioChannelSide::Center => (0f64, channel.factor, 0f64),
                AudioChannelSide::Left => (channel.factor, channel.factor, channel.factor),
                AudioChannelSide::NotLocalized => (0f64, 0f64, channel.factor),
                AudioChannelSide::Right => (channel.factor, 0f64, 0f64),
            });
        }
    }

    pub fn update_dimensions(&mut self, width: i32, height: i32) {
        // if the requested height is different from current height
        // it might be necessary to force rendering when stream
        // is paused or eos

        self.force_redraw |= self.req_width != width || self.req_height != height;

        if self.force_redraw {
            self.shareable_state_changed = true;
            self.is_ready = self.sample_step != 0;

            debug!("{}_upd.dim prev. f.redraw {}, w {}, h {}, sample_step_f. {}",
                self.id, self.force_redraw, self.req_width, self.req_height, self.sample_step_f
            );

            self.req_width = width;
            self.req_height = height;

            debug!("{}_new f.redraw {}, w {}, h {}, sample_step_f. {}",
                self.id, self.force_redraw, self.req_width, self.req_height, self.sample_step_f
            );
        }
    }

    pub fn update_sample_step(&mut self, sample_step_f: f64) {
        self.force_redraw |= (self.sample_step_f - sample_step_f).abs() > 0.01f64;
        self.is_ready = self.force_redraw && (self.req_width != 0);

        self.sample_step_f = sample_step_f;
        self.sample_step = (sample_step_f as usize).max(1);
        self.x_step_f = if sample_step_f < 1f64 {
            (1f64 / sample_step_f).round()
        } else {
            1f64
        };
        self.x_step = self.x_step_f as usize;

        self.shareable_state_changed = true;
    }

    pub fn is_ready(&self) -> bool {
        self.is_ready
    }

    pub fn get_image(&self) -> &cairo::ImageSurface {
        self.exposed_image.as_ref().unwrap()
    }

    pub fn update_from_other(&mut self, other: &mut WaveformImage) {
        if other.shareable_state_changed {
            if self.sample_step != other.sample_step || self.x_step != other.x_step {
                self.sample_step_f = other.sample_step_f;
                self.sample_step = other.sample_step;
                self.x_step_f = other.x_step_f;
                self.x_step = other.x_step;
                self.force_redraw = true;
            }

            self.req_width = other.req_width;
            self.req_height = other.req_height;

            other.shareable_state_changed = false;
        }
    }

    // Render the waveform within the provided limits.
    // This function is called from a working buffer
    // which means that self.exposed_image is the image
    // that was previously exposed to the UI.
    // This also means that we can safely deal with both
    // images since none of them is exposed at this very moment.
    // The rendering process reuses the previously rendered image
    // whenever possible.
    pub fn render(&mut self, audio_buffer: &AudioBuffer, lower: usize, upper: usize) {
        #[cfg(feature = "profile-waveform-image")]
        let start = Utc::now();

        // Align requested lower and upper sample bounds in order to keep
        // a steady offset between redraws. This allows using the same samples
        // for a given req_step_duration and avoiding flickering
        // between redraws.
        let mut lower = lower / self.sample_step * self.sample_step;
        let upper = upper / self.sample_step * self.sample_step;
        if audio_buffer.eos && upper == audio_buffer.upper
            || self.contains_eos
                && (upper == self.upper || (!self.force_redraw && lower >= self.lower))
        {
            // reached eos or image already contains eos and won't change
            if !self.contains_eos {
                debug!(concat!("{}_render setting contains_eos. ",
                        "Requested [{}, {}], current [{}, {}], force_redraw: {}",
                    ),
                    self.id,
                    lower,
                    upper,
                    self.lower,
                    self.upper,
                    self.force_redraw,
                );

                self.contains_eos = true;
            }
        } else if self.contains_eos {
            self.contains_eos = false;

            debug!(concat!("{}_render clearing contains_eos. ",
                    "Requested [{}, {}], current [{}, {}], force_redraw {} ",
                    "audio_buffer.eos {}",
                ),
                self.id,
                lower,
                upper,
                self.lower,
                self.upper,
                self.force_redraw,
                audio_buffer.eos,
            );
        }

        if lower < audio_buffer.lower {
            // first sample might be smaller than audio_buffer.lower
            // due to alignement on sample_step
            debug!("{}_render lower {} is less than buffer.lower {}",
                self.id, lower, audio_buffer.lower,
            );
            lower += self.sample_step;
        }

        if lower >= upper {
            // can't draw current range
            // reset WaveformImage state
            debug!("{}_render lower {} greater or equal upper {}", self.id, lower, upper);

            self.lower = 0;
            self.upper = 0;
            self.first = None;
            self.last = None;
            self.contains_eos = false;
            self.is_ready = false;
            return;
        }

        if upper < lower + 2 * self.sample_step {
            debug!("{}_render range [{}, {}] too small for sample_step: {}",
                self.id, lower, upper, self.sample_step,
            );
            return;
        }

        self.force_redraw |= !self.is_ready;

        if !self.force_redraw && lower >= self.lower && upper <= self.upper {
            // target extraction fits in previous extraction
            return;
        } else if upper < self.lower || lower > self.upper {
            // current samples extraction doesn't overlap with samples in previous image
            self.force_redraw = true;

            debug!(concat!("{}_render no overlap self.lower {}, ",
                    "self.upper {}, lower {}, upper {}",
                ),
                self.id, self.lower, self.upper, lower, upper,
            );
        }

        let (exposed_image, secondary_image) = {
            let target_width = if self.image_width > 0 {
                self.image_width
                    .max(((upper - lower) * self.x_step / self.sample_step) as i32)
            } else {
                INIT_WIDTH.max(((upper - lower) * self.x_step / self.sample_step) as i32)
            };
            if (target_width == self.image_width && self.req_height == self.image_height)
                || (self.force_redraw && target_width <= self.image_width
                    && self.req_height == self.image_height)
            {
                // expected dimensions fit in current image => reuse it
                (
                    self.exposed_image.take().unwrap(),
                    self.secondary_image.take().unwrap(),
                )
            } else {
                // can't reuse => create new images and force redraw
                self.force_redraw = true;
                self.image_width = target_width;
                self.image_width_f = f64::from(target_width);
                self.image_height = self.req_height;
                self.image_height_f = f64::from(self.req_height);

                debug!("{}_render new images w {}, h {}", self.id, target_width, self.req_height);

                (
                    cairo::ImageSurface::create(
                        // exposed_image
                        cairo::Format::Rgb24,
                        target_width,
                        self.req_height,
                    ).expect(&format!(
                        "WaveformBuffer.render: couldn't create image surface with width {}",
                        target_width,
                    )),
                    cairo::ImageSurface::create(
                        // will be used as secondary_image
                        cairo::Format::Rgb24,
                        target_width,
                        self.req_height,
                    ).unwrap(), // exposed_image could be created with same dimensions
                )
            }
        };

        if self.force_redraw {
            // Initialization or resolution has changed or seek requested
            // redraw the whole range from the audio buffer

            self.redraw(exposed_image, secondary_image, audio_buffer, lower, upper);
        } else if lower < self.lower {
            // can append samples before previous first sample
            if self.first.is_some() {
                // first sample position is known
                // shift previous image to the right
                // and append samples to the left
                self.append_left(exposed_image, secondary_image, audio_buffer, lower);
            } else {
                // first sample position is unknown
                // => force redraw
                warn!("{}_append left first sample unknown => redrawing", self.id);
                let sample_step = self.sample_step;
                self.redraw(
                    exposed_image,
                    secondary_image,
                    audio_buffer,
                    lower,
                    upper.min(audio_buffer.upper / sample_step * sample_step),
                );
            }
        } else if upper > self.upper {
            // can append samples after previous last sample
            if self.last.is_some() {
                // last sample position is known
                // shift previous image to the left (if necessary)
                // and append missing samples to the right

                // update lower in case a call to append_left
                // ends up adding nothing
                let lower = lower.max(self.lower);

                self.append_right(exposed_image, secondary_image, audio_buffer, lower, upper);
            } else {
                // last sample position is unknown
                // => force redraw
                warn!("{}_append right last sample unknown => redrawing", self.id);
                let sample_step = self.sample_step;
                self.redraw(
                    exposed_image,
                    secondary_image,
                    audio_buffer,
                    lower,
                    upper.min(audio_buffer.upper / sample_step * sample_step),
                );
            }
        }

        #[cfg(feature = "dump-waveform")]
        {
            let mut output_file = File::create(format!(
                "{}/waveform_{}_{}.png",
                WAVEFORM_DUMP_DIR,
                Utc::now().format("%H:%M:%S%.6f"),
                self.id,
            )).unwrap();
            self.exposed_image
                .as_ref()
                .unwrap()
                .write_to_png(&mut output_file)
                .unwrap();
        }

        self.is_ready = true;
    }

    // Redraw the whole sample range on a clean image
    fn redraw(
        &mut self,
        exposed_image: cairo::ImageSurface,
        secondary_image: cairo::ImageSurface,
        audio_buffer: &AudioBuffer,
        lower: usize,
        upper: usize,
    ) {
        let cr = cairo::Context::new(&exposed_image);
        self.exposed_image = Some(exposed_image);
        self.secondary_image = Some(secondary_image);

        cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
        cr.paint();

        self.set_scale(&cr);

        match self.draw_samples(&cr, audio_buffer, lower, upper, 0f64) {
            Some((first, last)) => {
                self.draw_amplitude_0(&cr, 0f64, last.x);

                self.first = Some(first);
                self.last = Some(last);

                self.lower = lower;
                self.upper = upper;
                self.force_redraw = false;
            }
            None => {
                self.force_redraw = true;
                warn!("{}_redraw: iter out of range {}, {}", self.id, lower, upper);
            }
        };

        debug!("{}_redraw smpl_stp {}, lower {}, upper {}",
            self.id, self.sample_step, self.lower, self.upper
        );
    }

    fn append_left(
        &mut self,
        exposed_image: cairo::ImageSurface,
        secondary_image: cairo::ImageSurface,
        audio_buffer: &AudioBuffer,
        lower: usize,
    ) {
        let sample_offset = self.lower - lower;
        let x_offset = (sample_offset / self.sample_step * self.x_step) as f64;

        #[cfg(test)]
        debug!("append_left x_offset {}, lower {}, self.lower {}, buffer.lower {}",
            x_offset, lower, self.lower, audio_buffer.lower
        );

        // translate exposed image on secondary_image
        // secondary_image becomes the exposed image
        let cr = cairo::Context::new(&secondary_image);

        self.translate_previous(&cr, &exposed_image, x_offset);

        self.exposed_image = Some(secondary_image);
        self.secondary_image = Some(exposed_image);

        self.set_scale(&cr);

        self.first = match self.first.take() {
            Some(mut first) => {
                first.x += x_offset;
                self.clear_area(&cr, 0f64, first.x);
                Some(first)
            }
            None => {
                self.clear_area(&cr, 0f64, x_offset);
                None
            }
        };
        self.last = match self.last.take() {
            Some(last) => {
                let next_last_pixel = last.x + x_offset;
                if next_last_pixel < self.image_width_f {
                    // last still in image
                    Some(WaveformSample {
                        x: next_last_pixel,
                        values: last.values,
                    })
                } else {
                    // last out of image
                    // get sample from previous image
                    // which is now bound to last pixel in current image
                    self.get_sample_and_values_at(
                        self.image_width_f - 1f64 - x_offset,
                        audio_buffer,
                    ).map(|(last_sample, values)| {
                        self.upper = audio_buffer.upper.min(last_sample + self.sample_step);
                        // align on the first pixel for the sample
                        let new_last_pixel =
                            ((self.image_width - 1) as usize / self.x_step * self.x_step) as f64;
                        self.clear_area(&cr, new_last_pixel, self.image_width_f);
                        WaveformSample {
                            x: new_last_pixel,
                            values: values,
                        }
                    })
                }
            }
            None => None,
        };

        if let Some((first_added, _last_added)) =
            self.draw_samples(&cr, audio_buffer, lower, self.lower, 0f64)
        {
            self.first = Some(first_added);
            self.lower = lower;
        } else {
            info!("{}_appd_left iter ({}, {}) out of range or too small",
                self.id, lower, self.lower
            );
        }

        #[cfg(test)]
        debug!("exiting append_left self.lower {}, self.upper {}", self.lower, self.upper);
    }

    fn append_right(
        &mut self,
        exposed_image: cairo::ImageSurface,
        secondary_image: cairo::ImageSurface,
        audio_buffer: &AudioBuffer,
        lower: usize,
        upper: usize,
    ) -> bool {
        let x_offset = ((lower - self.lower) / self.sample_step * self.x_step) as f64;

        #[cfg(test)]
        debug!(concat!("append_right x_offset {}, (lower {} upper {}), ",
                "self: (lower {} upper {}), buffer: (lower {}, upper {})",
            ),
            x_offset,
            lower,
            upper,
            self.lower,
            self.upper,
            audio_buffer.lower,
            audio_buffer.upper
        );

        let must_translate = match self.last.as_ref() {
            Some(last) => {
                let range_to_draw =
                    ((upper - self.upper.max(lower)) / self.sample_step * self.x_step) as f64;
                !(last.x + range_to_draw < self.image_width_f)
            }
            None => true,
        };

        let cr = if must_translate && x_offset > 0f64 {
            // translate exposed image on secondary_image
            // secondary_image becomes the exposed image
            let cr = cairo::Context::new(&secondary_image);

            self.translate_previous(&cr, &exposed_image, -x_offset);

            self.exposed_image = Some(secondary_image);
            self.secondary_image = Some(exposed_image);

            self.set_scale(&cr);

            self.lower = lower;
            self.first = audio_buffer.get(self.lower).map(|values| WaveformSample {
                x: 0f64,
                values: WaveformImage::convert_sample_values(values),
            });

            self.last = self.last.take().map(|last| {
                let new_last_x = last.x - x_offset;
                self.clear_area(&cr, new_last_x + 1f64, self.image_width_f);
                WaveformSample {
                    x: new_last_x,
                    values: last.values,
                }
            });

            cr
        } else {
            // Don't translate => reuse exposed image
            let cr = cairo::Context::new(&exposed_image);
            self.exposed_image = Some(exposed_image);
            self.secondary_image = Some(secondary_image);

            self.set_scale(&cr);

            cr
        };

        let first_sample_to_draw = self.upper.max(lower);
        let first_x_to_draw =
            ((first_sample_to_draw - self.lower) / self.sample_step * self.x_step) as f64;

        if let Some((_first_added, last_added)) = self.draw_samples(
            &cr,
            audio_buffer,
            first_sample_to_draw,
            upper,
            first_x_to_draw,
        ) {
            self.last = Some(last_added);
            self.upper = upper;
        } else {
            info!("{}_appd_right iter ({}, {}) out of range or too small",
                self.id, first_sample_to_draw, upper
            );
        }

        #[cfg(test)]
        debug!("exiting append_right self.lower {}, self.upper {}", self.lower, self.upper);

        true
    }

    fn get_sample_and_values_at(
        &self,
        x: f64,
        audio_buffer: &AudioBuffer,
    ) -> Option<(usize, Vec<f64>)> {
        let sample = self.lower + (x as usize) / self.x_step * self.sample_step;

        #[cfg(test)]
        {
            let values = match audio_buffer.get(sample) {
                Some(values) => format!("{:?}", values.to_vec()),
                None => "-".to_owned(),
            };
            debug!(concat!("WaveformImage{}_smpl_val_at {}, smpl {}, val {}, ",
                    "x step: {}, smpl step: {}, audiobuf. [{}, {}]",
                ),
                self.id,
                x,
                sample,
                values,
                self.x_step,
                self.sample_step,
                audio_buffer.lower,
                audio_buffer.upper
            );
        }

        audio_buffer
            .get(sample)
            .map(|values| (sample, WaveformImage::convert_sample_values(values)))
    }

    fn translate_previous(
        &self,
        cr: &cairo::Context,
        previous_image: &cairo::ImageSurface,
        x_offset: f64,
    ) {
        cr.scale(1f64, 1f64);
        cr.set_source_surface(previous_image, x_offset, 0f64);
        cr.paint();
    }

    fn set_scale(&self, cr: &cairo::Context) {
        cr.scale(1f64, self.image_height_f / *SAMPLES_RANGE);
    }

    fn set_channel_color(&self, cr: &cairo::Context, channel: usize) {
        if let Some(&(red, green, blue)) = self.channel_colors.get(channel) {
            cr.set_source_rgba(red, green, blue, 0.68f64);
        } else {
            warn!("{}_set_channel_color no color for channel {}", self.id, channel);
        }
    }

    fn link_samples(&self, cr: &cairo::Context, from: &WaveformSample, to: &WaveformSample) {
        #[cfg(test)]
        cr.set_source_rgb(0f64, 0.8f64, 0f64);

        for channel in 0..from.values.len() {
            #[cfg(not(test))]
            self.set_channel_color(cr, channel);

            cr.move_to(from.x, from.values[channel]);
            cr.line_to(to.x, to.values[channel]);
            cr.stroke();
        }
    }

    fn convert_sample(value: &i16) -> f64 {
        *SAMPLES_OFFSET - f64::from(*value) * *SAMPLES_SCALE_FACTOR
    }

    fn convert_sample_values(values: &[i16]) -> Vec<f64> {
        let mut result: Vec<f64> = Vec::with_capacity(values.len());
        for value in values {
            result.push(WaveformImage::convert_sample(value));
        }
        result
    }

    // Draw samples from sample_iter starting at first_x.
    // Returns the lower bound and last drawn coordinates.
    #[cfg_attr(feature = "cargo-clippy", allow(question_mark))]
    fn draw_samples(
        &self,
        cr: &cairo::Context,
        audio_buffer: &AudioBuffer,
        lower: usize,
        upper: usize,
        first_x: f64,
    ) -> Option<(WaveformSample, WaveformSample)> {
        if self.x_step == 1 {
            cr.set_line_width(1f64);
        } else if self.x_step < 4 {
            cr.set_line_width(1.5f64);
        } else {
            cr.set_line_width(2f64);
        }

        let sample_iter = audio_buffer.iter(lower, upper, self.sample_step);
        if sample_iter.is_none() {
            warn!(concat!("{}_draw_samples invalid iter for ",
                    "[{}, {}] sample_step {}, buffer: [{}, {}]",
                ),
                self.id,
                lower,
                upper,
                self.sample_step,
                audio_buffer.lower,
                audio_buffer.upper
            );

            return None;
        }

        let mut sample_iter = sample_iter.unwrap();
        if sample_iter.size_hint().0 < 2 {
            debug!(concat!("{}_draw_samples too small to render for ",
                    "[{}, {}] sample_step {}, buffer: [{}, {}]",
                ),
                self.id,
                lower,
                upper,
                self.sample_step,
                audio_buffer.lower,
                audio_buffer.upper
            );

            return None;
        }

        #[cfg(test)]
        {
            // in test mode, draw marks at
            // the start and end of each chunk
            cr.set_source_rgb(0f64, 0f64, 1f64);
            cr.move_to(first_x, 0f64);
            cr.line_to(first_x, *SAMPLES_OFFSET / 2f64);
            cr.stroke();
        }

        let mut first_values: Vec<f64> = Vec::with_capacity(audio_buffer.channels);
        let mut last_values: Vec<f64> = Vec::with_capacity(audio_buffer.channels);

        let sample = sample_iter.next();
        for channel_value in sample.unwrap() {
            let y = WaveformImage::convert_sample(channel_value);
            first_values.push(y);
            last_values.push(y);
        }

        let first_added = WaveformSample {
            x: first_x,
            values: first_values,
        };
        let first_for_amp0 = match self.last.as_ref() {
            Some(prev_last) => {
                if (first_x - prev_last.x).abs() <= self.x_step_f {
                    // appending samples right after previous last sample => add a link
                    self.link_samples(cr, prev_last, &first_added);
                    prev_last.x
                } else {
                    first_x
                }
            }
            None => first_x,
        };

        let mut x = first_x;
        for sample in sample_iter {
            let prev_x = x;
            x += self.x_step_f;

            for (channel, value) in sample.iter().enumerate() {
                self.set_channel_color(cr, channel);
                cr.move_to(prev_x, last_values[channel]);

                last_values[channel] = WaveformImage::convert_sample(value);
                cr.line_to(x, last_values[channel]);
                cr.stroke();
            }
        }

        let last_added = WaveformSample {
            x: x,
            values: last_values,
        };
        let last_for_amp0 = match self.first.as_ref() {
            Some(prev_first) => {
                if (prev_first.x - x).abs() <= self.x_step_f {
                    // appending samples right before previous first sample => add a link
                    self.link_samples(cr, &last_added, prev_first);
                    prev_first.x
                } else {
                    x
                }
            }
            None => x,
        };

        self.draw_amplitude_0(cr, first_for_amp0, last_for_amp0);

        #[cfg(test)]
        {
            // in test mode, draw marks at
            // the start and end of each chunk
            cr.set_source_rgb(1f64, 0f64, 1f64);
            cr.move_to(x, 1.5f64 * *SAMPLES_OFFSET);
            cr.line_to(x, *SAMPLES_RANGE);
            cr.stroke();
        }

        Some((first_added, last_added))
    }

    // clear samples previously rendered
    fn draw_amplitude_0(&self, cr: &cairo::Context, first_x: f64, last_x: f64) {
        cr.set_line_width(1f64);
        cr.set_source_rgb(
            AMPLITUDE_0_COLOR.0,
            AMPLITUDE_0_COLOR.1,
            AMPLITUDE_0_COLOR.2,
        );

        cr.move_to(first_x, *SAMPLES_OFFSET);
        cr.line_to(last_x, *SAMPLES_OFFSET);
        cr.stroke();
    }

    // clear samples previously rendered
    fn clear_area(&self, cr: &cairo::Context, first_x: f64, limit_x: f64) {
        cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
        cr.rectangle(first_x, 0f64, limit_x - first_x, *SAMPLES_RANGE);
        cr.fill();
    }
}

#[cfg(test)]
mod tests {
    //use env_logger;
    use gstreamer as gst;
    use gstreamer_audio as gst_audio;
    use gstreamer_audio::AUDIO_FORMAT_S16;

    use std::fs::{create_dir, File};
    use std::io::ErrorKind;

    use std::i16;

    use media::{AudioBuffer, AudioChannel, AudioChannelSide};
    use ui::WaveformImage;

    const OUT_DIR: &'static str = "target/test";
    const SAMPLE_RATE: u32 = 300;
    const SAMPLE_DYN: i32 = 300;

    fn prepare_tests() {
        match create_dir(&OUT_DIR) {
            Ok(_) => (),
            Err(error) => match error.kind() {
                ErrorKind::AlreadyExists => (),
                _ => panic!("WaveformImage test: couldn't create directory {}", OUT_DIR),
            },
        }
    }

    fn init(sample_step_f: f64, width: i32) -> (AudioBuffer, WaveformImage) {
        //env_logger::try_init();
        gst::init().unwrap();

        prepare_tests();

        // AudioBuffer
        let mut audio_buffer = AudioBuffer::new(1_000_000_000); // 1s
        audio_buffer.init(
            gst_audio::AudioInfo::new(AUDIO_FORMAT_S16, SAMPLE_RATE, 2).build().unwrap()
        );

        // WaveformImage
        let mut waveform = WaveformImage::new(0);
        waveform.set_channels(&[
            AudioChannel {
                side: AudioChannelSide::Left,
                factor: 1f64,
            },
            AudioChannel {
                side: AudioChannelSide::Right,
                factor: 1f64,
            },
        ]);
        waveform.update_dimensions(width, SAMPLE_DYN);
        waveform.update_sample_step(sample_step_f); // 1 sample / px

        (audio_buffer, waveform)
    }

    // Build a buffer with 2 channels in the specified range
    // which would be rendered as a diagonal on a Waveform image
    // from left top corner to right bottom of the target image
    // if all samples are rendered in the range [0:SAMPLE_RATE]
    fn build_buffer(lower_value: usize, upper_value: usize) -> Vec<i16> {
        let mut buffer: Vec<i16> = Vec::new();
        for index in lower_value..upper_value {
            let value = (index as f64 / SAMPLE_RATE as f64 * f64::from(i16::MAX)) as i16;
            buffer.push(value as i16);
            buffer.push(-(value as i16)); // second channel <= opposite value
        }
        buffer
    }

    fn render_with_samples(
        prefix: &str,
        waveform: &mut WaveformImage,
        audio_buffer: &mut AudioBuffer,
        incoming_samples: &[i16],
        lower: usize,
        is_new_segement: bool,
        sample_window: usize,
        can_scroll: bool,
    ) {
        info!("*** {}", prefix);

        let incoming_lower = lower;
        let incoming_upper = lower + incoming_samples.len() / audio_buffer.channels;

        audio_buffer.push_samples(incoming_samples, lower, is_new_segement);

        let (lower_to_extract, upper_to_extract) = if can_scroll {
            // scrolling is allowed
            // buffer fits in image completely
            if incoming_upper > waveform.upper {
                // incoming samples extend waveform on the right
                if incoming_lower > waveform.lower {
                    // incoming samples extend waveform on the right only
                    if audio_buffer.upper > audio_buffer.lower + sample_window {
                        (audio_buffer.upper - sample_window, audio_buffer.upper)
                    } else {
                        (audio_buffer.lower, audio_buffer.upper)
                    }
                } else {
                    // incoming samples extend waveform on both sides
                    if audio_buffer.upper > sample_window {
                        (audio_buffer.upper - sample_window, audio_buffer.upper)
                    } else {
                        (audio_buffer.lower, audio_buffer.upper)
                    }
                }
            } else {
                // incoming samples ends before current waveform's end
                if incoming_lower >= waveform.lower {
                    // incoming samples are contained in current waveform
                    (waveform.lower, waveform.upper)
                } else {
                    // incoming samples extend current waveform on the left only
                    (
                        incoming_lower,
                        waveform.upper.min(incoming_lower + sample_window),
                    )
                }
            }
        } else {
            // scrolling not allowed
            // => render a waveform that contains previous waveform
            //    + incoming sample
            (
                incoming_lower.min(waveform.lower),
                incoming_upper.max(waveform.upper),
            )
        };

        info!("rendering: [{}, {}] incoming [{}, {}]",
            lower_to_extract, upper_to_extract, incoming_lower, incoming_upper
        );
        waveform.render(&audio_buffer, lower_to_extract, upper_to_extract);

        let image = waveform.get_image();

        let mut output_file = File::create(format!("{}/waveform_image_{}_{:03}_{:03}.png",
            OUT_DIR, prefix, waveform.lower, waveform.upper
        )).unwrap();
        image.write_to_png(&mut output_file).unwrap();
    }

    #[test]
    fn additive_draws() {
        let (mut audio_buffer, mut waveform) = init(3f64, 250);
        let samples_window = SAMPLE_RATE as usize;

        render_with_samples(
            "additive_0 init",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(100, 200),
            100,
            true,
            samples_window,
            true,
        );
        render_with_samples(
            "additive_1 overlap on the left and on the right",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(50, 250),
            50,
            true,
            samples_window,
            true,
        );
        render_with_samples(
            "additive_2 overlap on the left",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(0, 100),
            0,
            true,
            samples_window,
            true,
        );
        render_with_samples(
            "additive_3 scrolling and overlap on the right",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(150, 340),
            150,
            true,
            samples_window,
            true,
        );
        render_with_samples(
            "additive_4 scrolling and overlaping on the right",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(0, 200),
            250,
            true,
            samples_window,
            true,
        );
    }

    #[test]
    fn link_between_draws() {
        let (mut audio_buffer, mut waveform) = init(1f64 / 5f64, 1480);
        let samples_window = SAMPLE_RATE as usize;

        render_with_samples(
            "link_0",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(100, 200),
            100,
            true,
            samples_window,
            true,
        );
        // append to the left
        render_with_samples(
            "link_1",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(25, 125),
            0,
            true,
            samples_window,
            true,
        );
        // appended to the right
        render_with_samples(
            "link_2",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(175, 275),
            200,
            true,
            samples_window,
            true,
        );
    }

    #[test]
    fn seek() {
        let (mut audio_buffer, mut waveform) = init(1f64, 300);
        let samples_window = SAMPLE_RATE as usize;

        render_with_samples(
            "seek_0",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(0, 100),
            100,
            true,
            samples_window,
            true,
        );
        // seeking forward
        render_with_samples(
            "seek_1",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(0, 100),
            500,
            true,
            samples_window,
            true,
        );
        // additional samples
        render_with_samples(
            "seek_2",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(100, 200),
            600,
            true,
            samples_window,
            true,
        );
        // additional samples
        render_with_samples(
            "seek_3",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(200, 300),
            700,
            true,
            samples_window,
            true,
        );
    }

    #[test]
    fn oveflow() {
        let (mut audio_buffer, mut waveform) = init(1f64 / 5f64, 1500);
        let samples_window = SAMPLE_RATE as usize;

        render_with_samples(
            "oveflow_0",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(0, 200),
            250,
            true,
            samples_window,
            true,
        );
        // overflow on the left
        render_with_samples(
            "oveflow_1",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(0, 300),
            0,
            true,
            samples_window,
            true,
        );
        // overflow on the right
        render_with_samples(
            "oveflow_2",
            &mut waveform,
            &mut audio_buffer,
            &build_buffer(0, 100),
            400,
            true,
            samples_window,
            true,
        );
    }
}
