extern crate cairo;

use std::i16;

#[cfg(feature = "profile-waveform-image")]
use chrono::Utc;

use media::{AudioBuffer, AudioChannel, AudioChannelSide};

pub const BACKGROUND_COLOR:  (f64, f64, f64) = (0.2f64, 0.2235f64, 0.2314f64);
pub const AMPLITUDE_0_COLOR: (f64, f64, f64) = (0.5f64, 0.5f64, 0f64);

// Initial image dimensions
// will dynamically adapt if needed
const INIT_WIDTH: i32 = 2000;
const INIT_HEIGHT: i32 = 500;

// Samples normalization factores
const SAMPLES_RANGE: f64 = INIT_HEIGHT as f64;
const SAMPLES_OFFSET: f64 = SAMPLES_RANGE / 2f64;
const SAMPLES_SCALE_FACTOR: f64 = SAMPLES_OFFSET / (i16::MAX as f64);

pub struct WaveformImage {
    pub id: usize,
    pub is_ready: bool,
    pub shareable_state_changed: bool,

    channel_colors: Vec<(f64, f64, f64)>,

    exposed_image: Option<cairo::ImageSurface>,
    working_image: Option<cairo::ImageSurface>,
    image_width: i32,
    image_width_f: f64,
    image_height: i32,
    image_height_f: f64,

    req_width: i32,
    req_height: i32,
    force_redraw: bool,

    pub lower: usize,
    pub upper: usize,

    first: Option<(f64, Vec<f64>)>,
    pub last: Option<(f64, Vec<f64>)>,

    pub sample_window: usize,
    pub sample_step_f: f64,
    pub sample_step: usize,
    x_step_f: f64,
    pub x_step: usize,
}

impl WaveformImage {
    pub fn new(id: usize) -> Self {
        WaveformImage {
            id: id,
            is_ready: false,
            shareable_state_changed: false,

            channel_colors: Vec::new(),

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
            image_width: 0,
            image_width_f: 0f64,
            image_height: 0,
            image_height_f: 0f64,

            req_width: 0,
            req_height: 0,
            force_redraw: false,

            lower: 0,
            upper: 0,

            first: None,
            last: None,

            sample_window: 0,
            sample_step_f: 0f64,
            sample_step: 0,
            x_step_f: 0f64,
            x_step: 0,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        self.is_ready = false;
        self.shareable_state_changed = false;

        self.channel_colors.clear();

        // self.exposed_image & self.working_image
        // will be cleaned on next with draw
        self.image_width = 0;
        self.image_width_f = 0f64;
        self.image_height = 0;
        self.image_height_f = 0f64;

        self.req_width = 0;
        self.req_height = 0;
        self.force_redraw = false;

        self.lower = 0;
        self.upper = 0;

        self.first = None;
        self.last = None;

        self.sample_window = 0;
        self.sample_step_f = 0f64;
        self.sample_step = 0;
        self.x_step_f = 0f64;
        self.x_step = 0;
    }

    pub fn set_channels(&mut self, channels: &[AudioChannel]) {
        #[cfg(feature = "trace-waveform-rendering")]
        println!("WaveformImage{}::set_channels {}", self.id, channels.len());

        for channel in channels {
            self.channel_colors.push(
                match channel.side {
                    AudioChannelSide::Center => {
                        (0f64, channel.factor, 0f64)
                    },
                    AudioChannelSide::Left => {
                        (channel.factor, channel.factor, channel.factor)
                    },
                    AudioChannelSide::NotLocalized => {
                        (0f64, 0f64, channel.factor)
                    },
                    AudioChannelSide::Right => {
                        (channel.factor, 0f64, 0f64)
                    },
                }
            );
        }
    }

    pub fn update_dimensions(&mut self, sample_step_f: f64, width: i32, height: i32) {
        // if the requested height is different from current height
        // it might be necessary to force rendering when stream
        // is paused or eos

        let force_redraw = self.sample_step_f != sample_step_f
            || self.req_width != width
            || self.req_height != height;

        if force_redraw {
            self.shareable_state_changed = true;

            #[cfg(feature = "trace-waveform-rendering")]
            println!("WaveformImage{}::upd.dim prev. f.redraw {}, w {}, h {}, sample_step_f. {}",
                self.id, self.force_redraw, self.req_width, self.req_height, self.sample_step_f
            );

            self.force_redraw = force_redraw;
            self.req_width = width;
            self.req_height = height;
            self.sample_step_f = sample_step_f;
            self.sample_step = (sample_step_f as usize).max(1);
            self.x_step_f =
                if sample_step_f < 1f64 {
                    (1f64 / sample_step_f).round()
                } else {
                    1f64
                };
            self.x_step = self.x_step_f as usize;

            #[cfg(feature = "trace-waveform-rendering")]
            println!("\t\t\tnew   f.redraw {}, w {}, h {}, sample_step_f. {}",
                self.force_redraw, self.req_width, self.req_height, self.sample_step_f
            );
        }
    }

    pub fn is_ready(&self) -> bool {
        self.is_ready
    }

    pub fn get_image(&self) -> &cairo::ImageSurface {
        self.exposed_image.as_ref().unwrap()
    }

    pub fn update_from_other(&mut self, other: &mut WaveformImage) {
        if other.shareable_state_changed {
            self.sample_step_f = other.sample_step_f;
            self.sample_step = other.sample_step;
            self.x_step_f = other.x_step_f;
            self.x_step = other.x_step;
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
    pub fn render(&mut self,
        audio_buffer: &AudioBuffer,
        lower: usize,
        upper: usize,
    ) {
        #[cfg(feature = "profile-waveform-image")]
        let start = Utc::now();

        if audio_buffer.samples.len() < self.sample_step {
            // buffer too small to render
            return;
        }

        // Align requested lower and upper sample bounds in order to keep
        // a steady offset between redraws. This allows using the same samples
        // for a given req_step_duration and avoiding flickering
        // between redraws.
        let mut lower =
            lower / self.sample_step * self.sample_step;
        let upper = upper / self.sample_step * self.sample_step;
        if lower < audio_buffer.lower {
            // first sample might be smaller than audio_buffer.lower
            // due to alignement on sample_step
            lower += self.sample_step;
        }

        if lower >= upper {
            // can't draw current range
            // reset WaveformImage state
            #[cfg(any(test, feature = "trace-waveform-rendering"))]
            println!("WaveformImage{}::render lower {} greater or equal upper {}",
                self.id, lower, upper
            );

            self.lower = 0;
            self.upper = 0;
            self.first = None;
            self.last = None;
            self.is_ready = false;
            return;
        }

        self.force_redraw |= !self.is_ready;

        if !self.force_redraw && lower >= self.lower
        && upper <= self.upper
        {   // traget extraction fits in previous extraction
            return;
        } else if upper < self.lower || lower > self.upper
        {   // current samples extraction doesn't overlap with samples in previous image
            self.force_redraw = true;

            #[cfg(any(test, feature = "trace-waveform-rendering"))]
            {
                println!("WaveformImage{}::render no overlap self.lower {}, self.upper {}",
                    self.id, self.lower, self.upper
                );
                println!("\t\t\tlower {}, upper {}", lower, upper);
            }
        }

        let (working_image, previous_image) = {
            let working_image = self.working_image.take().unwrap();

            let target_width = INIT_WIDTH
                .max(self.req_width)
                .max(((upper - lower) * self.x_step / self.sample_step) as i32);
            if (target_width == self.image_width
                && self.req_height == self.image_height)
            || (self.force_redraw
                && target_width <= self.image_width
                && self.req_height == self.image_height)
            {   // expected dimensions fit in current image => reuse it
                (
                    working_image,
                    self.exposed_image.take().unwrap(),
                )
            } else { // can't reuse => create new images and force redraw
                self.force_redraw = true;
                self.image_width = target_width;
                self.image_width_f = target_width as f64;
                self.image_height = self.req_height;
                self.image_height_f = self.req_height as f64;

                #[cfg(any(test, feature = "trace-waveform-rendering"))]
                println!("WaveformImage{}::render new images w {}, h {}",
                    self.id, target_width, self.req_height
                );

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

        if self.force_redraw {
            // Initialization or resolution has changed or seek requested
            // redraw the whole range from the audio buffer

            self.redraw(&cr, audio_buffer, lower, upper);
        } else {
            // can reuse previous context
            // Note: condition lower >= self.self.lower
            //              && upper <= self.self.upper
            // (traget extraction fits in previous extraction)
            // already checked

            let must_copy =
                if lower < self.lower {
                    // can append samples before previous first sample
                    if self.first.is_some() {
                        // first sample position is known
                        // shift previous image to the right
                        // and append samples to the left
                        self.append_left(&cr, &previous_image,
                            audio_buffer, lower
                        );
                    } else {
                        // first sample position is unknown
                        // => force redraw
                        println!("WaveformImage::append left({}): first sample unknown => redrawing", self.id);
                        let sample_step = self.sample_step;
                        self.redraw(&cr,
                            audio_buffer,
                            lower,
                            upper.min(
                                audio_buffer.upper / sample_step * sample_step
                            ),
                        );
                    }
                    false
                } else {
                    true
                };

            if upper > self.upper {
                // can append samples after previous last sample
                if self.last.is_some() {
                    // last sample position is known
                    // shift previous image to the left (if necessary)
                    // and append missing samples to the right
                    self.append_right(&cr,
                        &previous_image, must_copy,
                        audio_buffer, lower, upper
                    );
                } else {
                    // last sample position is unknown
                    // => force redraw
                    println!("WaveformImage::append right({}): last sample unknown => redrawing", self.id);
                    let sample_step = self.sample_step;
                    self.redraw(&cr,
                        audio_buffer,
                        lower,
                        upper.min(
                            audio_buffer.upper / sample_step * sample_step
                        ),
                    );
                }
            }
        }

        // swap images
        self.working_image = Some(previous_image);
        self.exposed_image = Some(working_image);

        self.sample_window = self.upper - self.lower;
        self.is_ready = true;
        self.force_redraw = false;

        #[cfg(feature = "profile-waveform-image")]
        let end = Utc::now();

        #[cfg(feature = "profile-waveform-image")]
        println!("waveform-image,{},{}",
            start.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
        );
    }

    // Redraw the whole sample range on a clean image
    fn redraw(&mut self,
        cr: &cairo::Context,
        audio_buffer: &AudioBuffer,
        lower: usize,
        upper: usize,
    ) {
        cr.set_source_rgb(
            BACKGROUND_COLOR.0,
            BACKGROUND_COLOR.1,
            BACKGROUND_COLOR.2
        );
        cr.paint();

        self.set_scale(&cr);

        match self.draw_samples(cr, audio_buffer, lower, upper, 0f64)
        {
            Some((first, (last_x, last_values))) => {
                self.draw_amplitude_0(cr, 0f64, last_x);

                self.first = Some(first);
                self.last = Some((last_x, last_values));

                self.lower = lower;
                self.upper = upper;
                self.force_redraw = false;
            },
            None => {
                self.force_redraw = true;
                println!("/!\\ WaveformImage{}::redraw: iter out of range {}, {}",
                    self.id, lower, upper
                );
            },
        };

        #[cfg(any(test, feature = "trace-waveform-rendering"))]
        {
            println!("WaveformImage{}::redraw smpl_stp {}, lower {}, upper {}",
                self.id, self.sample_step, self.lower, self.upper
            );
            self.trace_positions(&self.first, &self.last);
        }
    }

    fn append_left(&mut self,
        cr: &cairo::Context,
        previous_image: &cairo::ImageSurface,
        audio_buffer: &AudioBuffer,
        lower: usize,
    ) {
        let sample_offset = self.lower - lower;
        let x_offset =
            (
                sample_offset / self.sample_step * self.x_step
            ) as f64;

        #[cfg(test)]
        println!("append_left x_offset {}, lower {}, self.lower {}, buffer.lower {}",
            x_offset, lower, self.lower, audio_buffer.lower
        );

        self.translate_previous(cr, previous_image, x_offset);
        self.set_scale(&cr);

        self.first = match self.first.take() {
            Some((mut x, values)) => {
                x += x_offset;
                self.clear_area(&cr, 0f64, x);
                Some((x, values))
            },
            None => {
                self.clear_area(&cr, 0f64, x_offset);
                None
            },
        };
        self.last = match self.last.take() {
            Some((x, y)) => {
                let next_last_pixel = x + x_offset;
                if next_last_pixel < self.image_width_f {
                    // last still in image
                    Some((next_last_pixel, y))
                } else {
                    // last out of image
                    // get sample from previous image
                    // which is now bound to last pixel in current image
                    self.get_sample_and_values_at(
                        self.image_width_f - 1f64 - x_offset,
                        audio_buffer
                    ).map(|(last_sample, values)| {
                        self.upper = audio_buffer.upper.min(
                                last_sample + self.sample_step
                            );
                        // align on the first pixel for the sample
                        let new_last_pixel = (
                            (self.image_width - 1) as usize
                            / self.x_step * self.x_step
                        ) as f64;
                        self.clear_area(&cr, new_last_pixel, self.image_width_f);
                        (new_last_pixel, values)
                    })
                }
            },
            None => None,
        };

        match self.draw_samples(cr, audio_buffer, lower, self.lower, 0f64) {
            Some(((first_added_x, first_added_val), (last_added_x, last_added_val))) =>
            {
                if let Some((prev_first_x, prev_first_val)) = self.first.take() {
                    if (prev_first_x - last_added_x).abs() <= self.x_step_f {
                        // link new added samples with previous first sample
                        self.link_samples(&cr,
                            last_added_x, &last_added_val,
                            prev_first_x, &prev_first_val,
                        );
                    } else {
                        #[cfg(any(test, feature = "trace-waveform-rendering"))]
                        {
                            println!("/!\\ WaveformImage{}::appd_left can't link [{}, {}], last added ({}, {:?}), upper {}",
                                self.id, lower, self.lower, last_added_x, last_added_val, self.upper
                            );
                            self.trace_positions(
                                &Some((prev_first_x, prev_first_val)),
                                &self.last
                            );
                        }
                    }

                    self.draw_amplitude_0(cr, 0f64, prev_first_x);

                    self.first = Some((first_added_x, first_added_val));
                } else {
                    #[cfg(any(test, feature = "trace-waveform-rendering"))]
                    {
                        println!("/!\\ WaveformImage{}::appd_left no prev first [{}, {}], upper {}",
                            self.id, lower, self.lower, self.upper
                        );
                        self.trace_positions(&self.first, &self.last);
                    }
                }

                self.lower = lower;
            },
            None => {
                println!("/!\\ WaveformImage{}::appd_left iter ({}, {}) out of range or empty",
                    self.id, lower, self.lower
                );
            },
        };

        #[cfg(test)]
        {
            println!("exiting append_left self.lower {}, self.upper {}", self.lower, self.upper);
            self.trace_positions(&self.first, &self.last);
        }
    }

    fn append_right(&mut self,
        cr: &cairo::Context,
        previous_image: &cairo::ImageSurface,
        must_copy: bool,
        audio_buffer: &AudioBuffer,
        lower: usize,
        upper: usize,
    ) {
        let x_offset =
            ((lower - self.lower) / self.sample_step * self.x_step) as f64;

        #[cfg(test)]
        println!("append_right x_offset {}, (lower {} upper {}), self: (lower {} upper {}), buffer: (lower {}, upper {})",
            x_offset, lower, upper, self.lower, self.upper, audio_buffer.lower, audio_buffer.upper
        );

        if must_copy {
            self.translate_previous(cr, previous_image, -x_offset);
            self.set_scale(&cr);

            if x_offset > 0f64 {
                self.lower = lower;
                self.first = audio_buffer.get(self.lower)
                    .map(|values| {
                        (0f64, WaveformImage::convert_sample_values(values))
                    });

                self.last = self.last.take().map(|(x, values)| (x - x_offset, values));
            }
        }

        let first_sample_to_draw = self.upper.max(lower);
        let first_x_to_draw =
            ((first_sample_to_draw - self.lower) / self.sample_step * self.x_step) as f64;

        match self.last.as_ref() {
            Some(&(x, ref _values)) => self.clear_area(&cr, x, self.image_width_f),
            None => self.clear_area(&cr, first_x_to_draw, self.image_width_f),
        };

        match self.draw_samples(cr, audio_buffer, first_sample_to_draw, upper, first_x_to_draw) {
            Some(((first_added_x, first_added_val), (last_added_x, last_added_val))) =>
            {
                if let Some((prev_last_x, prev_last_val)) = self.last.take() {
                    if (first_added_x - prev_last_x).abs() <= self.x_step_f {
                        // link new added samples with previous last sample
                        self.link_samples(&cr,
                            prev_last_x, &prev_last_val,
                            first_added_x, &first_added_val,
                        );
                    } else {
                        #[cfg(any(test, feature = "trace-waveform-rendering"))]
                        {
                            println!("/!\\ WaveformImage{}::appd_right can't link [{}, {}], first_added ({}, {:?}), upper {}",
                                self.id, self.lower, lower, first_added_x, first_added_val, self.upper
                            );
                            self.trace_positions(
                                &self.first,
                                &Some((prev_last_x, prev_last_val))
                            );
                        }
                    }

                    self.draw_amplitude_0(cr, prev_last_x, last_added_x);

                    self.last = Some((last_added_x, last_added_val));
                } else {
                    #[cfg(any(test, feature = "trace-waveform-rendering"))]
                    {
                        println!("/!\\ WaveformImage{}::appd_right no prev last self.lower {}, lower {}, self.upper {}, upper {}",
                            self.id, self.lower, lower, self.upper, upper
                        );
                        self.trace_positions(&self.first, &self.last);
                    }
                }

                self.upper = upper;
            },
            None => {
                println!("/!\\ WaveformImage{}::appd_right iter ({}, {}) out of range or empty",
                    self.id, first_sample_to_draw, upper
                );
            },
        };

        #[cfg(test)]
        {
            println!("exiting append_right self.lower {}, self.upper {}", self.lower, self.upper);
            self.trace_positions(&self.first, &self.last);
        }
    }

    fn get_sample_and_values_at(&self,
        x: f64,
        audio_buffer: &AudioBuffer
    ) -> Option<(usize, Vec<f64>)> {
        let sample = self.lower + (x as usize) / self.x_step * self.sample_step;

        #[cfg(test)]
        {
            let values = match audio_buffer.get(sample) {
                Some(values) => format!("{:?}", values.to_vec()),
                None => "-".to_owned(),
            };
            println!("WaveformImage{}::smpl_val_at {}, smpl {}, val {}, x step: {}, smpl step: {}, audiobuf. [{}, {}]",
                self.id, x, sample, values, self.x_step, self.sample_step, audio_buffer.lower, audio_buffer.upper
            );
        }

        audio_buffer.get(sample).map(|values|
            (sample, WaveformImage::convert_sample_values(values))
        )
    }

    #[cfg(any(test, feature = "trace-waveform-rendering"))]
    fn trace_positions(&self,
        first: &Option<(f64, Vec<f64>)>,
        last: &Option<(f64, Vec<f64>)>
    ) {
        let first = match first.as_ref() {
            Some(&(x, ref values)) => format!("({}, {:?})", x, values),
            None => "-".to_owned(),
        };
        let last = match last.as_ref() {
            Some(&(x, ref values)) => format!("({}, {:?})", x, values),
            None => "-".to_owned(),
        };
        println!("\tx_step {}, first {}, last {}", self.x_step, first, last);
    }

    fn translate_previous(&self,
        cr: &cairo::Context,
        previous_image: &cairo::ImageSurface,
        x_offset: f64
    ) {
        cr.scale(1f64, 1f64);
        cr.set_source_surface(previous_image, x_offset, 0f64);
        cr.paint();
    }

    fn set_scale(&self, cr: &cairo::Context) {
        cr.scale(1f64, self.image_height_f / SAMPLES_RANGE);
    }

    fn set_channel_color(&self, cr: &cairo::Context, channel: usize) {
        if let Some(&(red, green, blue)) = self.channel_colors.get(channel) {
            cr.set_source_rgba(red, green, blue, 0.68f64);
        }
    }

    fn link_samples(&self,
        cr: &cairo::Context,
        from_x: f64, from_values: &[f64],
        to_x: f64, to_values: &[f64],
    ) {
        #[cfg(test)]
        cr.set_source_rgb(0f64, 0.8f64, 0f64);

        for channel in 0..from_values.len() {
            #[cfg(not(test))]
            self.set_channel_color(cr, channel);

            cr.move_to(from_x, from_values[channel]);
            cr.line_to(to_x, to_values[channel]);
            cr.stroke();
        }
    }

    fn convert_sample(value: i16) -> f64 {
        SAMPLES_OFFSET - f64::from(value) * SAMPLES_SCALE_FACTOR
    }

    fn convert_sample_values(values: &[i16]) -> Vec<f64> {
        let mut result: Vec<f64> = Vec::with_capacity(values.len());
        for value in values {
            result.push(WaveformImage::convert_sample(*value));
        }
        result
    }

    // Draw samples from sample_iter starting at first_x.
    // Returns the lower bound and last drawn coordinates.
    fn draw_samples(&self,
        cr: &cairo::Context,
        audio_buffer: &AudioBuffer,
        lower: usize,
        upper: usize,
        first_x: f64,
    ) -> Option<((f64, Vec<f64>), (f64, Vec<f64>))>
    {
        if self.x_step == 1 {
            cr.set_line_width(1f64);
        } else if self.x_step < 4 {
            cr.set_line_width(1.5f64);
        } else {
            cr.set_line_width(2f64);
        }

        #[cfg(test)]
        {   // in test mode, draw marks at
            // the start and end of each chunk
            cr.set_source_rgb(0f64, 0f64, 1f64);
            cr.move_to(first_x, 0f64);
            cr.line_to(first_x, SAMPLES_OFFSET / 2f64);
            cr.stroke();
        }

        let mut first_values: Vec<f64> = Vec::with_capacity(audio_buffer.channels);
        let mut last_values: Vec<f64> = Vec::with_capacity(audio_buffer.channels);

        let mut x = first_x;
        for channel in 0..audio_buffer.channels {
            let mut sample_iter =
                match audio_buffer.iter(lower, upper, self.sample_step, channel) {
                    Some(sample_iter) =>
                        if sample_iter.size_hint().0 > 0 {
                            sample_iter
                        } else {
                            #[cfg(any(test, feature = "trace-waveform-rendering"))]
                            {
                                println!("/!\\ WaveformImage{}::draw_samples empty buffer lower {}, upper {}, sample_step {}, channel {}",
                                    self.id, lower, upper, self.sample_step, channel
                                );
                            }
                            return None;
                        },
                    None => {
                        #[cfg(any(test, feature = "trace-waveform-rendering"))]
                        {
                            println!("/!\\ WaveformImage{}::draw_samples invalid iter lower {}, upper {}, sample_step {}, channel {}",
                                self.id, lower, upper, self.sample_step, channel
                            );
                        }
                        return None
                    },
                };

            self.set_channel_color(cr, channel);

            x = first_x;
            let mut sample_value =
                WaveformImage::convert_sample(sample_iter.next().unwrap());
            first_values.push(sample_value);

            for sample in sample_iter {
                cr.move_to(x, sample_value);
                x += self.x_step_f;
                sample_value = WaveformImage::convert_sample(sample);
                cr.line_to(x, sample_value);
                cr.stroke();
            }

            #[cfg(any(test, feature = "trace-waveform-rendering"))]
            {
                if x == first_x {
                    println!("/!\\ WaveformImage{}::draw_samples only one sample lower {}, upper {}, sample_step {}, channel {}",
                        self.id, lower, upper, self.sample_step, channel
                    );
                }
            }

            last_values.push(sample_value);
        }

        #[cfg(test)]
        {   // in test mode, draw marks at
            // the start and end of each chunk
            cr.set_source_rgb(1f64, 0f64, 1f64);
            cr.move_to(x, 1.5f64 * SAMPLES_OFFSET);
            cr.line_to(x, SAMPLES_RANGE);
            cr.stroke();
        }

        Some(((first_x, first_values), (x, last_values)))
    }

    // clear samples previously rendered
    fn draw_amplitude_0(&self, cr: &cairo::Context, first_x: f64, last_x: f64) {
        cr.set_line_width(1f64);
        cr.set_source_rgb(
            AMPLITUDE_0_COLOR.0,
            AMPLITUDE_0_COLOR.1,
            AMPLITUDE_0_COLOR.2
        );

        cr.move_to(first_x, SAMPLES_OFFSET);
        cr.line_to(last_x, SAMPLES_OFFSET);
        cr.stroke();
    }

    // clear samples previously rendered
    fn clear_area(&self, cr: &cairo::Context, first_x: f64, limit_x: f64) {
        cr.set_source_rgb(
            BACKGROUND_COLOR.0,
            BACKGROUND_COLOR.1,
            BACKGROUND_COLOR.2
        );
        cr.rectangle(first_x, 0f64, limit_x - first_x, SAMPLES_RANGE);
        cr.fill();
    }
}

#[cfg(test)]
mod tests {
    extern crate cairo;

    extern crate gstreamer as gst;

    use std::fs::{create_dir, File};
    use std::io::ErrorKind;

    use std::i16;

    use media::{AudioBuffer, AudioChannel, AudioChannelSide};
    use ui::WaveformImage;

    const OUT_DIR: &'static str = "target/test";
    const SAMPLE_RATE: u64 = 300;
    const SAMPLE_DYN:  i32 = 300;

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

    fn init(sample_step_f: f64, width: i32) -> (AudioBuffer, WaveformImage) {
        gst::init().unwrap();

        prepare_tests();

        // AudioBuffer
        let mut audio_buffer = AudioBuffer::new(1_000_000_000); // 1s
        audio_buffer.init(SAMPLE_RATE, 2); //2 channels

        // WaveformImage
        let mut waveform = WaveformImage::new(0);
        waveform.set_channels(&[
            AudioChannel{side: AudioChannelSide::Left, factor: 1f64},
            AudioChannel{side: AudioChannelSide::Right, factor: 1f64}
        ]);
        waveform.update_dimensions(
            sample_step_f, // 1 sample / px
            width,
            SAMPLE_DYN
        );

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
        incoming_samples: Vec<i16>,
        lower: usize,
        segement_lower: usize,
        sample_window: usize,
        can_scroll: bool,
    ) {
        println!("\n*** {}", prefix);

        let incoming_lower = lower;
        let incoming_upper = lower + incoming_samples.len() / audio_buffer.channels;

        audio_buffer.push_samples(
            &incoming_samples,
            lower,
            segement_lower,
        );

        let (lower_to_extract, upper_to_extract) =
            if can_scroll {
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
                        (incoming_lower, waveform.upper.min(incoming_lower + sample_window))
                    }
                }
            } else {
                // scrolling not allowed
                // => render a waveform that contains previous waveform
                //    + incoming sample
                (incoming_lower.min(waveform.lower), incoming_upper.max(waveform.upper))
            };


        println!("Rendering: [{}, {}] incoming [{}, {}]",
            lower_to_extract, upper_to_extract, incoming_lower, incoming_upper
        );
        waveform.render(&audio_buffer,
            lower_to_extract,
            upper_to_extract,
        );

        let image = waveform.get_image();

        let mut output_file = File::create(
                format!(
                    "{}/waveform_image_{}_{:03}_{:03}.png", OUT_DIR,
                    prefix,
                    waveform.lower,
                    waveform.upper
                )
            ).expect("WaveformImage test: couldn't create output file");
        image.write_to_png(&mut output_file)
            .expect("WaveformImage test: couldn't write waveform image");
    }

    #[test]
    fn additive_draws() {
        let (mut audio_buffer, mut waveform) = init(3f64, 250);
        let samples_window = SAMPLE_RATE as usize;

        render_with_samples("additive_0 init", &mut waveform, &mut audio_buffer,
            build_buffer(100, 200), 100, 100, samples_window, true);
        render_with_samples("additive_1 overlap on the left and on the right",
            &mut waveform, &mut audio_buffer,
            build_buffer(50, 250), 50, 50, samples_window, true);
        render_with_samples("additive_2 overlap on the left",
            &mut waveform, &mut audio_buffer,
            build_buffer(0, 100), 0, 0, samples_window, true);
        render_with_samples("additive_3 scrolling and overlap on the right",
            &mut waveform, &mut audio_buffer,
            build_buffer(150, 340), 150, 150, samples_window, true);
        render_with_samples("additive_4 scrolling and overlaping on the right",
            &mut waveform, &mut audio_buffer,
              build_buffer(0, 200), 250, 250, samples_window, true);
    }

    #[test]
    fn link_between_draws() {
        let (mut audio_buffer, mut waveform) = init(1f64 / 5f64, 1480);
        let samples_window = SAMPLE_RATE as usize;

        render_with_samples("link_0", &mut waveform, &mut audio_buffer,
            build_buffer(100, 200), 100, 100, samples_window, true);
        // append to the left
        render_with_samples("link_1", &mut waveform, &mut audio_buffer,
            build_buffer(25, 125), 0, 0, samples_window, true);
        // appended to the right
        render_with_samples("link_2", &mut waveform, &mut audio_buffer,
            build_buffer(175, 275), 200, 200, samples_window, true);
    }

    #[test]
    fn seek() {
        let (mut audio_buffer, mut waveform) = init(1f64, 300);
        let samples_window = SAMPLE_RATE as usize;

        render_with_samples("seek_0", &mut waveform, &mut audio_buffer,
            build_buffer(0, 100), 100, 100, samples_window, true);
        // seeking forward
        render_with_samples("seek_1", &mut waveform, &mut audio_buffer,
            build_buffer(0, 100), 500, 500, samples_window, true);
        // additional samples
        render_with_samples("seek_2", &mut waveform, &mut audio_buffer,
            build_buffer(100, 200), 600, 600, samples_window, true);
        // additional samples
        render_with_samples("seek_3", &mut waveform, &mut audio_buffer,
            build_buffer(200, 300), 700, 700, samples_window, true);
    }

    #[test]
    fn oveflow() {
        let (mut audio_buffer, mut waveform) = init(1f64 / 5f64, 1500);
        let samples_window = SAMPLE_RATE as usize;

        render_with_samples("oveflow_0", &mut waveform, &mut audio_buffer,
            build_buffer(0, 200), 250, 250, samples_window, true);
        // overflow on the left
        render_with_samples("oveflow_1", &mut waveform, &mut audio_buffer,
            build_buffer(0, 300), 0, 0, samples_window, true);
        // overflow on the right
        render_with_samples("oveflow_2", &mut waveform, &mut audio_buffer,
            build_buffer(0, 100), 400, 400, samples_window, true);
    }
}
