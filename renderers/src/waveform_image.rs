use cairo;
use log::{debug, info, warn};
use smallvec::SmallVec;

#[cfg(feature = "dump-waveform")]
use chrono::Utc;

#[cfg(feature = "dump-waveform")]
use std::{
    fs::{create_dir, File},
    io::ErrorKind,
};

use media::{
    AudioBuffer, AudioChannel, AudioChannelSide, SampleIndex, SampleIndexRange, SampleValue,
    INLINE_CHANNELS,
};

use super::Image;

pub const BACKGROUND_COLOR: (f64, f64, f64) = (0.2f64, 0.2235f64, 0.2314f64);
pub const AMPLITUDE_0_COLOR: (f64, f64, f64) = (0.5f64, 0.5f64, 0f64);

// Initial image dimensions
// will dynamically adapt if needed
const INIT_WIDTH: i32 = 2000;
const INIT_HEIGHT: i32 = 500;

#[cfg(feature = "dump-waveform")]
const WAVEFORM_DUMP_DIR: &str = "target/waveforms";

pub struct WaveformSample {
    pub x: f64,
    pub y_values: SmallVec<[f64; INLINE_CHANNELS]>,
}

pub struct WaveformImage {
    pub id: usize,
    pub is_initialized: bool,
    pub is_ready: bool,
    pub shareable_state_changed: bool,

    channel_colors: SmallVec<[(f64, f64, f64); INLINE_CHANNELS]>,

    exposed_image: Option<Image>,
    secondary_image: Option<Image>,
    image_width: i32,
    image_width_f: f64,
    image_height: i32,
    half_range_y: f64,
    full_range_y: f64,
    sample_value_factor: f64,

    req_width: i32,
    req_height: i32,
    force_redraw: bool,

    pub lower: SampleIndex,
    pub upper: SampleIndex,

    pub contains_eos: bool,

    first: Option<WaveformSample>,
    pub last: Option<WaveformSample>,

    pub sample_step_f: f64,
    pub sample_step: SampleIndexRange,
    x_step_f: f64,
    pub x_step: usize,
}

impl WaveformImage {
    pub fn new(id: usize) -> Self {
        #[cfg(feature = "dump-waveform")]
        let _ = create_dir(&WAVEFORM_DUMP_DIR).map_err(|err| match err.kind() {
            ErrorKind::AlreadyExists => (),
            _ => panic!(
                "WaveformImage::new couldn't create directory {}",
                WAVEFORM_DUMP_DIR
            ),
        });

        WaveformImage {
            id,
            is_initialized: false,
            is_ready: false,
            shareable_state_changed: false,

            channel_colors: SmallVec::new(),

            exposed_image: Some(
                Image::try_new(INIT_WIDTH, INIT_HEIGHT).expect("Default `WaveformImage`"),
            ),
            secondary_image: Some(
                Image::try_new(INIT_WIDTH, INIT_HEIGHT).expect("Default `WaveformImage`"),
            ),
            image_width: 0,
            image_width_f: 0f64,
            image_height: 0,
            half_range_y: 0f64,
            full_range_y: 0f64,
            sample_value_factor: 0f64,

            req_width: 0,
            req_height: 0,
            force_redraw: false,

            lower: SampleIndex::default(),
            upper: SampleIndex::default(),

            contains_eos: false,

            first: None,
            last: None,

            sample_step_f: 0f64,
            sample_step: SampleIndexRange::default(),
            x_step_f: 0f64,
            x_step: 0,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        debug!("{}_cleanup", self.id);

        // self.exposed_image & self.secondary_image
        // will be cleaned on next with draw
        self.is_initialized = false;
        self.image_width = 0;
        self.image_width_f = 0f64;
        self.image_height = 0;
        self.half_range_y = 0f64;
        self.full_range_y = 0f64;
        self.sample_value_factor = 0f64;

        self.req_width = 0;
        self.req_height = 0;

        self.cleanup_sample_conditions();
    }

    pub fn cleanup_sample_conditions(&mut self) {
        debug!("{}_cleanup_sample_conditions", self.id);
        self.is_ready = false;
        self.force_redraw = false;

        self.channel_colors.clear();

        self.lower = SampleIndex::default();
        self.upper = SampleIndex::default();

        self.contains_eos = false;

        self.first = None;
        self.last = None;

        self.sample_step_f = 0f64;
        self.sample_step = SampleIndexRange::default();
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
            self.is_initialized = self.sample_step != SampleIndexRange::default();

            debug!(
                "{}_upd.dim prev. f.redraw {}, w {}, h {}, sample_step_f. {}",
                self.id, self.force_redraw, self.req_width, self.req_height, self.sample_step_f
            );

            self.req_width = width;
            self.req_height = height;

            debug!(
                "{}_new f.redraw {}, w {}, h {}, sample_step_f. {}",
                self.id, self.force_redraw, self.req_width, self.req_height, self.sample_step_f
            );
        }
    }

    pub fn update_sample_step(&mut self, sample_step_f: f64) {
        self.force_redraw |= (self.sample_step_f - sample_step_f).abs() > 0.01f64;
        self.is_initialized = self.force_redraw && (self.req_width != 0);

        self.sample_step_f = sample_step_f;
        self.sample_step = (sample_step_f as usize).max(1).into();
        self.x_step_f = if sample_step_f < 1f64 {
            (1f64 / sample_step_f).round()
        } else {
            1f64
        };
        self.x_step = self.x_step_f as usize;

        self.shareable_state_changed = true;
    }

    pub fn get_image(&mut self) -> &mut Image {
        self.exposed_image.as_mut().unwrap()
    }

    pub fn update_from_other(&mut self, other: &mut WaveformImage) {
        if other.shareable_state_changed {
            debug!("{}_update_from_other shareable_state_changed", self.id);
            if self.sample_step != other.sample_step || self.x_step != other.x_step {
                self.sample_step_f = other.sample_step_f;
                self.sample_step = other.sample_step;
                self.x_step_f = other.x_step_f;
                self.x_step = other.x_step;
                self.is_initialized = other.is_initialized;
                self.force_redraw = true;
            }

            if self.req_width != other.req_width {
                self.req_width = other.req_width;
                self.force_redraw = true;
            }
            if self.req_height != other.req_height {
                self.req_height = other.req_height;
                self.force_redraw = true;
            }

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
    pub fn render(&mut self, audio_buffer: &AudioBuffer, lower: SampleIndex, upper: SampleIndex) {
        if self.req_width == 0 {
            debug!("{}_render not ready yet (self.req_width == 0)", self.id);
            return;
        }

        // Align requested lower and upper sample bounds in order to keep
        // a steady offset between redraws. This allows using the same samples
        // for a given req_step_duration and avoiding flickering
        // between redraws.
        let mut lower = lower.get_aligned(self.sample_step);
        let upper = upper.get_aligned(self.sample_step);
        if audio_buffer.contains_eos() && upper + self.sample_step > audio_buffer.upper
            || self.contains_eos
                && (upper == self.upper || (!self.force_redraw && lower >= self.lower))
        {
            // reached eos or image already contains eos and won't change
            if !self.contains_eos {
                debug!(
                    concat!(
                        "{}_render setting contains_eos. ",
                        "Requested [{}, {}], current [{}, {}], force_redraw: {}",
                    ),
                    self.id, lower, upper, self.lower, self.upper, self.force_redraw,
                );

                self.contains_eos = true;
            }
        } else if self.contains_eos {
            self.contains_eos = false;

            debug!(
                concat!(
                    "{}_render clearing contains_eos. ",
                    "Requested [{}, {}], current [{}, {}], force_redraw {} ",
                    "audio_buffer.eos {}",
                ),
                self.id,
                lower,
                upper,
                self.lower,
                self.upper,
                self.force_redraw,
                audio_buffer.contains_eos(),
            );
        }

        if lower < audio_buffer.lower {
            // first sample might be smaller than audio_buffer.lower
            // due to alignement on sample_step
            lower += self.sample_step;
        }

        if lower >= upper {
            // can't draw current range
            // reset WaveformImage state
            debug!(
                "{}_render lower {} greater or equal upper {}",
                self.id, lower, upper
            );

            self.lower = SampleIndex::default();
            self.upper = SampleIndex::default();
            self.first = None;
            self.last = None;
            self.contains_eos = false;
            self.is_ready = false;
            return;
        }

        if upper < lower + self.sample_step {
            debug!(
                "{}_render range [{}, {}] too small for sample_step: {}",
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

            debug!(
                concat!(
                    "{}_render no overlap self.lower {}, ",
                    "self.upper {}, lower {}, upper {}",
                ),
                self.id, self.lower, self.upper, lower, upper,
            );
        }

        let (exposed_image, secondary_image) = {
            let target_width = if self.image_width > 0 {
                self.image_width
                    .max(((upper - lower).get_step_range(self.sample_step) * self.x_step) as i32)
            } else {
                INIT_WIDTH
                    .max(((upper - lower).get_step_range(self.sample_step) * self.x_step) as i32)
            };
            if (target_width == self.image_width && self.req_height == self.image_height)
                || (self.force_redraw
                    && target_width <= self.image_width
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
                self.full_range_y = f64::from(self.req_height);
                self.half_range_y = self.full_range_y / 2f64;
                self.sample_value_factor = self.half_range_y / f64::from(std::i16::MIN);

                debug!(
                    "{}_render new images w {}, h {}",
                    self.id, target_width, self.req_height
                );

                (
                    // exposed_image
                    Image::try_new(target_width, self.req_height).unwrap_or_else(|err| {
                        panic!(
                            "WaveformBuffer.render creating {}x{} image: {}",
                            target_width, self.req_height, err,
                        )
                    }),
                    // will be used as secondary_image
                    Image::try_new(target_width, self.req_height).unwrap_or_else(|err| {
                        panic!(
                            "WaveformBuffer.render creating {}x{} image: {}",
                            target_width, self.req_height, err,
                        )
                    }),
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
                    upper.min(audio_buffer.upper.get_aligned(sample_step)),
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
                    upper.min(audio_buffer.upper.get_aligned(sample_step)),
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
            ))
            .unwrap();
            self.exposed_image
                .as_mut()
                .unwrap()
                .with_surface(|surface| {
                    surface.write_to_png(&mut output_file).unwrap();
                });
        }

        self.is_ready = true;
    }

    // Redraw the whole sample range on a clean image
    fn redraw(
        &mut self,
        mut exposed_image: Image,
        secondary_image: Image,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
    ) {
        exposed_image.with_surface(|image_surface| {
            let cr = cairo::Context::new(&image_surface);

            cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
            cr.paint();

            match self.draw_samples(&cr, audio_buffer, lower, upper, 0f64) {
                Some((first, last)) => {
                    self.first = Some(first);
                    self.last = Some(last);

                    self.lower = lower;
                    self.upper = upper;
                    self.force_redraw = false;
                }
                None => {
                    self.force_redraw = true;
                    warn!(
                        "{}_redraw: not enough samples to draw {}, {}",
                        self.id, lower, upper
                    );
                }
            }
        });

        debug!(
            "{}_redraw smpl_stp {}, lower {}, upper {}",
            self.id, self.sample_step, self.lower, self.upper
        );

        self.exposed_image = Some(exposed_image);
        self.secondary_image = Some(secondary_image);
    }

    fn append_left(
        &mut self,
        mut exposed_image: Image,
        mut secondary_image: Image,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
    ) {
        let sample_offset = self.lower - lower;
        let x_offset = (sample_offset.get_step_range(self.sample_step) * self.x_step) as f64;

        #[cfg(test)]
        debug!(
            "append_left x_offset {}, lower {}, self.lower {}, buffer.lower {}",
            x_offset, lower, self.lower, audio_buffer.lower
        );

        secondary_image.with_surface(|secondary_surface| {
            // translate exposed image on secondary_image
            let cr = cairo::Context::new(&secondary_surface);

            exposed_image.with_surface_external_context(&cr, |cr, exposed_surface| {
                self.translate_previous(cr, &exposed_surface, x_offset);
            });

            match self.first.as_mut() {
                Some(mut first) => {
                    first.x += x_offset;
                    let new_first_x = first.x;
                    self.clear_area(&cr, 0f64, new_first_x);
                }
                None => self.clear_area(&cr, 0f64, x_offset),
            };

            self.last = match self.last.take() {
                Some(last) => {
                    let next_last_pixel = last.x + x_offset;
                    if next_last_pixel < self.image_width_f {
                        // last still in image
                        Some(WaveformSample {
                            x: next_last_pixel,
                            y_values: last.y_values,
                        })
                    } else {
                        // last out of image
                        // get sample from previous image
                        // which is now bound to last pixel in current image
                        let last_x = self.image_width_f - 1f64 - x_offset;
                        let last_sample_idx =
                            self.lower + self.sample_step.get_scaled(last_x as usize, self.x_step);

                        let new_last_x =
                            ((self.image_width - 1) as usize / self.x_step * self.x_step) as f64;

                        self.upper = audio_buffer.upper.min(last_sample_idx + self.sample_step);

                        audio_buffer.get(last_sample_idx).map(|sample_values| {
                            self.clear_area(&cr, new_last_x, self.image_width_f);

                            WaveformSample {
                                x: new_last_x,
                                y_values: self.sample_values_to_ys(sample_values),
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
                info!(
                    "{}_appd_left iter ({}, {}) out of range or too small",
                    self.id, lower, self.lower
                );
            }
        });

        #[cfg(test)]
        debug!(
            "exiting append_left self.lower {}, self.upper {}",
            self.lower, self.upper
        );

        // secondary_image becomes the exposed image
        self.exposed_image = Some(secondary_image);
        self.secondary_image = Some(exposed_image);
    }

    fn append_right(
        &mut self,
        mut exposed_image: Image,
        mut secondary_image: Image,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
    ) {
        let x_offset = ((lower - self.lower).get_step_range(self.sample_step) * self.x_step) as f64;

        #[cfg(test)]
        debug!(
            concat!(
                "append_right x_offset {}, (lower {} upper {}), ",
                "self: (lower {} upper {}), buffer: (lower {}, upper {})",
            ),
            x_offset, lower, upper, self.lower, self.upper, audio_buffer.lower, audio_buffer.upper
        );

        let must_translate = self.last.as_ref().map_or(false, |last| {
            let x_range_to_draw =
                (upper - self.upper).get_step_range(self.sample_step) * self.x_step;
            last.x as usize + x_range_to_draw >= self.image_width as usize
        });

        if must_translate {
            // translate exposed image on secondary_image
            // secondary_image becomes the exposed image
            secondary_image.with_surface(|secondary_surface| {
                let cr = cairo::Context::new(&secondary_surface);

                exposed_image.with_surface_external_context(&cr, |cr, exposed_surface| {
                    self.translate_previous(cr, &exposed_surface, -x_offset);
                });

                self.lower = lower;
                self.first = audio_buffer.get(lower).map(|values| WaveformSample {
                    x: 0f64,
                    y_values: self.sample_values_to_ys(values),
                });

                if let Some(last) = self.last.as_mut() {
                    last.x -= x_offset;
                    let new_last_x = last.x;
                    self.clear_area(&cr, new_last_x + 1f64, self.image_width_f);
                }

                self.draw_right(&cr, audio_buffer, upper);
            });

            self.exposed_image = Some(secondary_image);
            self.secondary_image = Some(exposed_image);
        } else {
            // Don't translate => reuse exposed image
            exposed_image.with_surface(|exposed_surface| {
                let cr = cairo::Context::new(&exposed_surface);
                self.draw_right(&cr, audio_buffer, upper);
            });

            self.exposed_image = Some(exposed_image);
            self.secondary_image = Some(secondary_image);
        }

        #[cfg(test)]
        debug!(
            "exiting append_right self.lower {}, self.upper {}",
            self.lower, self.upper
        );
    }

    fn draw_right(&mut self, cr: &cairo::Context, audio_buffer: &AudioBuffer, upper: SampleIndex) {
        let first_sample_to_draw = self.upper;
        let first_x_to_draw = ((first_sample_to_draw - self.lower).get_step_range(self.sample_step)
            * self.x_step) as f64;

        if let Some((_first_added, last_added)) = self.draw_samples(
            &cr,
            audio_buffer,
            first_sample_to_draw,
            upper,
            first_x_to_draw,
        ) {
            debug_assert!(last_added.x < self.image_width_f);
            self.last = Some(last_added);
            self.upper = upper;
        } else {
            info!(
                "{}_appd_right iter ({}, {}) too small",
                self.id, first_sample_to_draw, upper
            );
        }
    }

    #[inline]
    fn translate_previous(
        &self,
        cr: &cairo::Context,
        previous_image: &cairo::ImageSurface,
        x_offset: f64,
    ) {
        cr.set_source_surface(previous_image, x_offset, 0f64);
        cr.paint();
    }

    #[inline]
    fn sample_value_to_y(&self, value: SampleValue) -> f64 {
        f64::from(i32::from(value.as_i16()) - i32::from(std::i16::MAX)) * self.sample_value_factor
    }

    #[inline]
    fn sample_values_to_ys(&self, values: &[SampleValue]) -> SmallVec<[f64; INLINE_CHANNELS]> {
        let mut result: SmallVec<[f64; INLINE_CHANNELS]> = SmallVec::with_capacity(values.len());
        for value in values {
            result.push(self.sample_value_to_y(*value));
        }

        result
    }

    // Draw samples from sample_iter starting at first_x.
    // Returns the lower bound and last drawn coordinates.
    #[allow(clippy::collapsible_if)]
    fn draw_samples(
        &self,
        cr: &cairo::Context,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
        first_x: f64,
    ) -> Option<(WaveformSample, WaveformSample)> {
        if self.x_step == 1 {
            cr.set_line_width(1f64);
        } else if self.x_step < 4 {
            cr.set_line_width(1.5f64);
        } else {
            cr.set_line_width(2f64);
        }

        #[cfg(test)]
        {
            // in test mode, draw marks at
            // the start and end of each chunk
            cr.set_source_rgb(0f64, 0f64, 1f64);
            cr.move_to(first_x, 0f64);
            cr.line_to(first_x, 0.5f64 * self.half_range_y);
            cr.stroke();
        }

        let mut first_y_values: SmallVec<[f64; INLINE_CHANNELS]> =
            SmallVec::with_capacity(audio_buffer.channels);
        let mut last_y_values: SmallVec<[f64; INLINE_CHANNELS]> =
            SmallVec::with_capacity(audio_buffer.channels);

        let (first_for_amp0, can_link_first) =
            self.last.as_ref().map_or((first_x, false), |prev_last| {
                if first_x > prev_last.x {
                    // appending samples after previous last sample => link
                    (prev_last.x, true)
                } else {
                    (first_x, false)
                }
            });

        let mut last_for_amp0 = first_for_amp0;
        let mut x = first_x;
        for channel in 0..audio_buffer.channels {
            let mut y_iter = audio_buffer
                .try_iter(lower, upper, channel, self.sample_step)
                .unwrap_or_else(|err| panic!("{}_draw_samples: {}", self.id, err))
                .map(|channel_value| self.sample_value_to_y(channel_value));

            if channel == 0 {
                if y_iter.size_hint().0 < 2 && !self.contains_eos {
                    debug!(
                        concat!(
                            "{}_draw_samples too small to render for ",
                            "[{}, {}], channel {} sample_step {}, buffer: [{}, {}]",
                        ),
                        self.id,
                        lower,
                        upper,
                        channel,
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
                    cr.line_to(first_x, 0.5f64 * self.half_range_y);
                    cr.stroke();
                }
            }

            if let Some(&(red, green, blue)) = self.channel_colors.get(channel) {
                cr.set_source_rgb(red, green, blue);
            } else {
                warn!(
                    "{}_set_channel_color no color for channel {}",
                    self.id, channel
                );
            }

            x = first_x;

            let y = y_iter
                .next()
                .unwrap_or_else(|| panic!("no first value for channel {}", channel));

            if can_link_first {
                // appending samples after previous last sample => link
                if let Some(prev_last) = self.last.as_ref() {
                    cr.move_to(prev_last.x, prev_last.y_values[channel]);
                    cr.line_to(x, y);
                }
            } else {
                cr.move_to(x, y);
            }

            first_y_values.push(y);

            // draw the rest of the samples
            let mut last_y = 0f64;
            for y in y_iter {
                x += self.x_step_f;
                cr.line_to(x, y);
                last_y = y;
            }

            last_y_values.push(last_y);

            // Link with previous first if applicable
            last_for_amp0 = self.first.as_ref().map_or(x, |prev_first| {
                if x < prev_first.x {
                    cr.line_to(prev_first.x, prev_first.y_values[channel]);

                    prev_first.x
                } else {
                    x
                }
            });

            cr.stroke();
        }

        #[cfg(test)]
        {
            // in test mode, draw marks at
            // the start and end of each chunk
            cr.set_source_rgb(1f64, 0f64, 1f64);
            cr.move_to(x, 1.5f64 * self.half_range_y);
            cr.line_to(x, self.full_range_y);
            cr.stroke();
        }

        // Draw axis
        cr.set_line_width(1f64);
        cr.set_source_rgb(
            AMPLITUDE_0_COLOR.0,
            AMPLITUDE_0_COLOR.1,
            AMPLITUDE_0_COLOR.2,
        );

        cr.move_to(first_for_amp0, self.half_range_y);
        cr.line_to(last_for_amp0, self.half_range_y);
        cr.stroke();

        Some((
            WaveformSample {
                x: first_x,
                y_values: first_y_values,
            },
            WaveformSample {
                x,
                y_values: last_y_values,
            },
        ))
    }

    // clear samples previously rendered
    fn clear_area(&self, cr: &cairo::Context, first_x: f64, limit_x: f64) {
        cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
        cr.rectangle(first_x, 0f64, limit_x - first_x, self.full_range_y);
        cr.fill();
    }
}

#[cfg(test)]
mod tests {
    //use env_logger;
    use byteorder::{ByteOrder, LittleEndian};
    use gstreamer as gst;
    use gstreamer_audio as gst_audio;
    use gstreamer_audio::AUDIO_FORMAT_S16;
    use log::info;

    use std::fs::{create_dir, File};
    use std::io::ErrorKind;

    use media::{
        AudioBuffer, AudioChannel, AudioChannelSide, Duration, SampleIndex, SampleIndexRange,
    };

    use super::WaveformImage;

    const OUT_DIR: &str = "../target/test";
    const SAMPLE_RATE: u32 = 300;
    const SAMPLE_DURATION: Duration = Duration::from_frequency(SAMPLE_RATE as u64);
    const SAMPLE_WINDOW: SampleIndexRange = SampleIndexRange::new(300);
    const SAMPLE_DYN: i32 = 300;

    const CHANNELS: usize = 2;

    fn prepare_tests() {
        let _ = create_dir(&OUT_DIR).map_err(|err| match err.kind() {
            ErrorKind::AlreadyExists => (),
            _ => panic!("WaveformImage test: couldn't create directory {}", OUT_DIR),
        });
    }

    fn init(sample_step_f: f64, width: i32) -> (AudioBuffer, WaveformImage) {
        //env_logger::init();
        gst::init().unwrap();

        prepare_tests();

        // AudioBuffer
        let mut audio_buffer = AudioBuffer::new(Duration::from_secs(1));
        audio_buffer.init(
            gst_audio::AudioInfo::new(AUDIO_FORMAT_S16, SAMPLE_RATE, CHANNELS as u32)
                .build()
                .unwrap(),
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
    fn build_buffer(lower_value: usize, upper_value: usize) -> gst::Buffer {
        let samples_u8_len = (upper_value - lower_value) * CHANNELS * 2;

        let mut buffer = gst::Buffer::with_size(samples_u8_len).unwrap();
        {
            let buffer_mut = buffer.get_mut().unwrap();

            let mut buffer_map = buffer_mut.map_writable().unwrap();
            let buffer_slice = buffer_map.as_mut();

            let mut buf_u8 = [0; CHANNELS];
            for index in lower_value..upper_value {
                for channel in 0..CHANNELS {
                    let mut value =
                        (index as f64 / SAMPLE_RATE as f64 * f64::from(std::i16::MAX)) as i16;
                    if channel != 0 {
                        value = -value;
                    }

                    LittleEndian::write_i16(&mut buf_u8, value);
                    let offset = (((index - lower_value) * CHANNELS) + channel) * 2;
                    buffer_slice[offset] = buf_u8[0];
                    buffer_slice[offset + 1] = buf_u8[1];
                }
            }
        }

        buffer
    }

    fn push_test_buffer(
        audio_buffer: &mut AudioBuffer,
        mut buffer: gst::Buffer,
        segment_lower: SampleIndex,
    ) {
        let pts = segment_lower.get_ts(SAMPLE_DURATION);
        {
            let buffer_mut = buffer.get_mut().unwrap();
            buffer_mut.set_pts(gst::ClockTime::from_nseconds(pts.as_u64()));
        }

        audio_buffer.have_gst_segment(pts);
        audio_buffer.push_gst_buffer(&buffer, SampleIndex::default()); // never drain buffer in this test
    }

    fn render(
        prefix: &str,
        waveform: &mut WaveformImage,
        audio_buffer: &mut AudioBuffer,
        buffer: gst::Buffer,
        lower: SampleIndex,
    ) {
        let incoming_lower = lower;
        let incoming_upper = lower + SampleIndexRange::new(buffer.get_size() / CHANNELS / 2);

        push_test_buffer(audio_buffer, buffer, lower);

        let (lower_to_extract, upper_to_extract) = if incoming_upper > waveform.upper {
            // incoming samples extend waveform on the right
            if incoming_lower > waveform.lower {
                // incoming samples extend waveform on the right only
                if audio_buffer.upper > audio_buffer.lower + SAMPLE_WINDOW {
                    (audio_buffer.upper - SAMPLE_WINDOW, audio_buffer.upper)
                } else {
                    (audio_buffer.lower, audio_buffer.upper)
                }
            } else {
                // incoming samples extend waveform on both sides
                if audio_buffer.upper > SAMPLE_WINDOW {
                    (audio_buffer.upper - SAMPLE_WINDOW, audio_buffer.upper)
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
                    waveform.upper.min(incoming_lower + SAMPLE_WINDOW),
                )
            }
        };

        info!(
            "rendering: [{}, {}] incoming [{}, {}]",
            lower_to_extract, upper_to_extract, incoming_lower, incoming_upper
        );
        waveform.render(&audio_buffer, lower_to_extract, upper_to_extract);

        let lower = waveform.lower;
        let upper = waveform.upper;
        let image = waveform.get_image();

        let mut output_file = File::create(format!(
            "{}/waveform_image_{}_{}_{}.png",
            OUT_DIR, prefix, lower, upper
        ))
        .unwrap();
        image.with_surface(|surface| {
            surface.write_to_png(&mut output_file).unwrap();
        });
    }

    #[test]
    fn additive_draws() {
        let (mut audio_buffer, mut waveform) = init(3f64, 250);

        render(
            "additive_0 init",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(100, 200),
            SampleIndex::new(100),
        );
        render(
            "additive_1 overlap on the left and on the right",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(50, 250),
            SampleIndex::new(50),
        );
        render(
            "additive_2 overlap on the left",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(0, 100),
            SampleIndex::new(0),
        );
        render(
            "additive_3 scrolling and overlap on the right",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(150, 340),
            SampleIndex::new(150),
        );
        render(
            "additive_4 scrolling and overlaping on the right",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(0, 200),
            SampleIndex::new(250),
        );
    }

    #[test]
    fn link_between_draws() {
        let (mut audio_buffer, mut waveform) = init(1f64 / 5f64, 1480);

        render(
            "link_0",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(100, 200),
            SampleIndex::new(100),
        );
        // append to the left
        render(
            "link_1",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(25, 125),
            SampleIndex::new(0),
        );
        // appended to the right
        render(
            "link_2",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(175, 275),
            SampleIndex::new(200),
        );
    }

    #[test]
    fn seek() {
        let (mut audio_buffer, mut waveform) = init(1f64, 300);

        render(
            "seek_0",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(0, 100),
            SampleIndex::new(100),
        );
        // seeking forward
        render(
            "seek_1",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(0, 100),
            SampleIndex::new(500),
        );
        // additional samples
        render(
            "seek_2",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(100, 200),
            SampleIndex::new(600),
        );
        // additional samples
        render(
            "seek_3",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(200, 300),
            SampleIndex::new(700),
        );
    }

    #[test]
    fn oveflow() {
        let (mut audio_buffer, mut waveform) = init(1f64 / 5f64, 1500);

        render(
            "oveflow_0",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(0, 200),
            SampleIndex::new(250),
        );
        // overflow on the left
        render(
            "oveflow_1",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(0, 300),
            SampleIndex::new(0),
        );
        // overflow on the right
        render(
            "oveflow_2",
            &mut waveform,
            &mut audio_buffer,
            build_buffer(0, 100),
            SampleIndex::new(400),
        );
    }
}
