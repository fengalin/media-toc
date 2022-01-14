use log::debug;
use smallvec::{smallvec, SmallVec};

use crate::{
    AudioBuffer, AudioChannel, AudioChannelSide, SampleIndex, SampleIndexRange, INLINE_CHANNELS,
};

use std::sync::{Arc, Mutex};

use super::{super::Image, Dimensions};

pub const BACKGROUND_COLOR: (f64, f64, f64) = (0.2f64, 0.2235f64, 0.2314f64);
pub const AXIS_COLOR: (f64, f64, f64) = (0.5f64, 0.5f64, 0f64);

// Translating samples in the negative range when scaling for display
// improves the rendering bench by 10%
const SAMPLE_AMPLITUDE: i32 = std::i16::MAX as i32;
const SAMPLE_RANGE: f64 = 2f64 * (std::i16::MIN as f64);

// Initial image dimensions
// will dynamically adapt if needed
const INIT_WIDTH: i32 = 2000;
const INIT_HEIGHT: i32 = 500;

#[derive(Debug, Default)]
pub struct WaveformSample {
    pub x: f64,
    pub y_values: SmallVec<[f64; INLINE_CHANNELS]>,
}

#[derive(Debug)]
pub struct ChannelColors(SmallVec<[(f64, f64, f64); INLINE_CHANNELS]>);

impl Default for ChannelColors {
    fn default() -> Self {
        ChannelColors(SmallVec::<[(f64, f64, f64); INLINE_CHANNELS]>::with_capacity(0))
    }
}

#[derive(Debug, Default)]
pub struct WaveformImage {
    pub id: usize,
    pub is_ready: bool,

    image_width: i32,
    image_width_f: f64,

    image_height: i32,
    half_range_y: f64,
    full_range_y: f64,
    sample_display_scale: f64,

    pub lower: SampleIndex,
    pub upper: SampleIndex,

    pub contains_eos: bool,

    last: WaveformSample,

    channel_colors: Arc<Mutex<ChannelColors>>,

    exposed_image: Option<Image>,
    // This one is only used by the working WaveformImage (the one on which we execute render).
    // Locking the Mutex should be cheap since there shouldn't be any contention.
    secondary_image: Arc<Mutex<Option<Image>>>,
}

impl WaveformImage {
    pub fn new(
        id: usize,
        channel_colors: Arc<Mutex<ChannelColors>>,
        secondary_image: Arc<Mutex<Option<Image>>>,
    ) -> Self {
        let exposed_image =
            Some(Image::try_new(INIT_WIDTH, INIT_HEIGHT).expect("Default `WaveformImage`"));

        {
            let mut secondary_image_opt = secondary_image.lock().unwrap();
            if secondary_image_opt.is_none() {
                *secondary_image_opt =
                    Some(Image::try_new(INIT_WIDTH, INIT_HEIGHT).expect("Default `WaveformImage`"));
            }
        }

        WaveformImage {
            id,
            exposed_image,
            secondary_image,
            channel_colors,
            ..WaveformImage::default()
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
        self.half_range_y = 0f64;
        self.full_range_y = 0f64;
        self.sample_display_scale = 0f64;

        self.cleanup_sample_conditions();
    }

    pub fn cleanup_sample_conditions(&mut self) {
        debug!("{}_cleanup_sample_conditions", self.id);
        self.is_ready = false;

        self.lower = SampleIndex::default();
        self.upper = SampleIndex::default();

        self.contains_eos = false;

        self.last = WaveformSample::default();
    }

    pub fn set_channels(&self, channels: impl Iterator<Item = AudioChannel>) {
        let mut channel_colors = self.channel_colors.lock().unwrap();

        channel_colors.0.clear();
        for channel in channels {
            debug!("{}_set_channels {:?}", self.id, channel.side);
            channel_colors.0.push(match channel.side {
                AudioChannelSide::Center => (0f64, channel.factor, 0f64),
                AudioChannelSide::Left => (channel.factor, channel.factor, channel.factor),
                AudioChannelSide::NotLocalized => (0f64, 0f64, channel.factor),
                AudioChannelSide::Right => (channel.factor, 0f64, 0f64),
            });
        }
    }

    pub fn image(&self) -> &Image {
        self.exposed_image.as_ref().unwrap()
    }

    // Render the waveform within the provided limits.
    // This function is called from a working buffer
    // which means that self.exposed_image is the image
    // that was previously exposed to the UI.
    // This also means that we can safely deal with both
    // images since none of them is exposed at this very moment.
    // The rendering process reuses the previously rendered image
    // whenever possible.
    pub fn render(
        &mut self,
        d: Dimensions,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
    ) {
        if d.sample_step == SampleIndexRange::default() {
            debug!("{}_render not ready yet {:#?}", self.id, d);
            return;
        }

        // Snap requested lower and upper sample bounds to sample_step in order to keep
        // a steady offset between redraws. This allows using the same samples
        // for a given req_step_duration and avoiding flickering
        // between redraws.
        // FIXME we should use the div_??? crate and snap to
        // ceil fpr lower or floor for upper
        let mut lower = lower.snap_to(d.sample_step);
        let upper = upper.snap_to(d.sample_step);
        let mut force_redraw = if self.id == 1 {
            d.force_redraw_1
        } else {
            d.force_redraw_2
        };
        if audio_buffer.contains_eos() && upper + d.sample_step > audio_buffer.upper
            || self.contains_eos && (upper == self.upper || (!force_redraw && lower >= self.lower))
        {
            // reached eos or image already contains eos and won't change
            if !self.contains_eos {
                debug!(
                    concat!(
                        "{}_render setting contains_eos. ",
                        "Requested [{}, {}], current [{}, {}], force_redraw: {}",
                    ),
                    self.id, lower, upper, self.lower, self.upper, force_redraw,
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
                force_redraw,
                audio_buffer.contains_eos(),
            );
        }

        // FIXME is this still needed or snap_to could take care of this?
        if lower < audio_buffer.lower {
            // first sample might be smaller than audio_buffer.lower
            // due to alignement on sample_step
            lower += d.sample_step;
        }

        if upper < lower + d.sample_step {
            debug!(
                "{}_render range [{}, {}] too small for sample_step: {}",
                self.id, lower, upper, d.sample_step,
            );
            return;
        }

        force_redraw |= !self.is_ready;

        if upper <= self.lower || lower >= self.upper {
            force_redraw = true;

            debug!(
                "{}_render forcing redraw image [{}, {}], requested [{}, {}] ",
                self.id, self.lower, self.upper, lower, upper,
            );
        }

        if !force_redraw && self.lower < upper && upper <= self.upper {
            // target extraction fits in previous extraction
            return;
        }

        let (exposed_image, secondary_image) = {
            let target_width = if self.image_width > 0 {
                self.image_width
                    .max(((upper - lower).step_range(d.sample_step) * d.x_step) as i32)
            } else {
                INIT_WIDTH.max(((upper - lower).step_range(d.sample_step) * d.x_step) as i32)
            };
            if (target_width == self.image_width && d.req_height == self.image_height)
                || (force_redraw
                    && target_width <= self.image_width
                    && d.req_height == self.image_height)
            {
                // expected dimensions fit in current image => reuse it
                (
                    self.exposed_image.take().unwrap(),
                    self.secondary_image.lock().unwrap().take().unwrap(),
                )
            } else {
                // can't reuse => create new images and force redraw
                force_redraw = true;
                self.image_width = target_width;
                self.image_width_f = f64::from(target_width);
                self.image_height = d.req_height;
                self.full_range_y = f64::from(d.req_height);
                self.half_range_y = self.full_range_y / 2f64;
                self.sample_display_scale = self.full_range_y / SAMPLE_RANGE;

                debug!(
                    "{}_render new images w {}, h {}",
                    self.id, target_width, d.req_height
                );

                // Release previous exposed image
                let _ = self.exposed_image.take().unwrap();
                // then, build a new one
                let exposed_image =
                    Image::try_new(target_width, d.req_height).unwrap_or_else(|err| {
                        panic!(
                            "WaveformBuffer.render creating {}x{} image: {}",
                            target_width, d.req_height, err,
                        )
                    });

                // Secondary image might have already been resized by the other WaveformImage
                let mut secondary_image = self.secondary_image.lock().unwrap().take().unwrap();
                if secondary_image.width != target_width || secondary_image.height != d.req_height {
                    secondary_image =
                        Image::try_new(target_width, d.req_height).unwrap_or_else(|err| {
                            panic!(
                                "WaveformBuffer.render creating {}x{} image: {}",
                                target_width, d.req_height, err,
                            )
                        })
                }

                (exposed_image, secondary_image)
            }
        };

        if !force_redraw {
            // append samples after previous last sample
            // shift previous image to the left (if necessary)
            // and append missing samples to the right
            self.append_right(
                &d,
                exposed_image,
                secondary_image,
                audio_buffer,
                lower,
                upper,
            );
        } else {
            self.redraw(
                &d,
                exposed_image,
                secondary_image,
                audio_buffer,
                lower,
                upper,
            );
            // the appropriate `p.force_redraw_n` flag was reset in caller.
        }

        self.is_ready = true;
    }

    // Redraw the whole sample range on a clean image
    fn redraw(
        &mut self,
        d: &Dimensions,
        exposed_image: Image,
        secondary_image: Image,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
    ) {
        self.last.x = 0f64;
        self.last.y_values = smallvec![self.half_range_y; audio_buffer.channels];

        exposed_image.with_surface(|image_surface| {
            let cr = cairo::Context::new(image_surface);

            cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
            cr.paint();

            self.draw_samples(d, &cr, audio_buffer, lower, upper);
        });

        debug!(
            "{}_redraw smpl_stp {}, lower {}, upper {}",
            self.id, d.sample_step, lower, upper
        );

        self.exposed_image = Some(exposed_image);
        *self.secondary_image.lock().unwrap() = Some(secondary_image);
        self.lower = lower;
        self.upper = upper;
    }

    fn append_right(
        &mut self,
        d: &Dimensions,
        exposed_image: Image,
        secondary_image: Image,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
    ) {
        let x_offset =
            (lower.saturating_sub(self.lower).step_range(d.sample_step) * d.x_step) as f64;

        let x_range_to_draw = (upper - self.upper).step_range(d.sample_step) * d.x_step;
        let must_translate = self.last.x as usize + x_range_to_draw >= self.image_width as usize;

        if must_translate {
            // translate exposed image on secondary_image
            // secondary_image becomes the exposed image
            secondary_image.with_surface(|secondary_surface| {
                let cr = cairo::Context::new(secondary_surface);

                exposed_image.with_surface_external_context(&cr, |cr, exposed_surface| {
                    cr.set_source_surface(exposed_surface, -x_offset, 0f64);
                    cr.paint();
                });

                self.lower = lower;

                self.last.x -= x_offset;
                self.clear_area(&cr, self.last.x, self.image_width_f);

                self.draw_samples(d, &cr, audio_buffer, self.upper, upper)
            });

            self.exposed_image = Some(secondary_image);
            *self.secondary_image.lock().unwrap() = Some(exposed_image);
        } else {
            // Don't translate => reuse exposed image
            exposed_image.with_surface(|exposed_surface| {
                let cr = cairo::Context::new(exposed_surface);
                self.draw_samples(d, &cr, audio_buffer, self.upper, upper)
            });

            self.exposed_image = Some(exposed_image);
            *self.secondary_image.lock().unwrap() = Some(secondary_image);
        }
    }

    /// Draws samples from sample_iter starting at first_x.
    #[allow(clippy::many_single_char_names)]
    fn draw_samples(
        &mut self,
        d: &Dimensions,
        cr: &cairo::Context,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
    ) {
        if d.x_step == 1 {
            cr.set_line_width(1f64);
        } else if d.x_step < 4 {
            cr.set_line_width(1.5f64);
        } else {
            cr.set_line_width(2f64);
        }

        #[cfg(test)]
        {
            // in test mode, draw marks at
            // the start and end of each chunk
            cr.set_source_rgb(0f64, 0f64, 1f64);
            cr.move_to(self.last.x + d.x_step_f, 0f64);
            cr.line_to(self.last.x + d.x_step_f, 0.5f64 * self.half_range_y);
            cr.stroke();
        }

        // There are two approches to trace the waveform:
        //
        // 1. Iterate on each channel and `stroke` all the samples for the given
        // channel at once. This is fast, but it produces artifacts at the jonction
        // with previously rendered samples. These artifacts are noticeable because
        // they appear at different timestamps in the 2 `WaveformRenderers` that
        // alternate in the double buffering mechanism.
        // 2. Iterate on each sample and trace the channels for current sample.
        // This is 25% slower than (1) since it involves more `stroke`s, but it
        // doesn't show the artifacts.
        //
        // Selected approach (2) because artifacts give a cheap impression.

        let sample_display_scale = self.sample_display_scale;
        let samples_iter = audio_buffer
            .try_iter(lower, upper, d.sample_step)
            .unwrap_or_else(|err| panic!("{}_draw_samples: {}", self.id, err));
        let start_x = self.last.x;

        let channel_colors = self.channel_colors.lock().unwrap();

        for samples in samples_iter {
            let y_iter = samples.iter().map(|sample| {
                f64::from(i32::from(sample.as_i16()) - SAMPLE_AMPLITUDE) * sample_display_scale
            });

            let x = self.last.x + d.x_step_f;
            for (channel, y) in y_iter.enumerate() {
                let (r, g, b) = channel_colors
                    .0
                    .get(channel)
                    .unwrap_or_else(|| panic!("no color for channel {}", channel));
                cr.set_source_rgb(*r, *g, *b);

                cr.move_to(self.last.x, self.last.y_values[channel]);
                cr.line_to(x, y);
                cr.stroke();

                self.last.y_values[channel] = y;
            }

            self.last.x = x;
        }

        drop(channel_colors);

        #[cfg(test)]
        {
            // in test mode, draw marks at
            // the start and end of each chunk
            cr.set_source_rgb(1f64, 0f64, 1f64);
            cr.move_to(self.last.x, 1.5f64 * self.half_range_y);
            cr.line_to(self.last.x, self.full_range_y);
            cr.stroke();
        }

        // FIXME: draw axis first (get x range from samples_iter)
        // Draw the axis
        cr.set_line_width(1f64);
        cr.set_source_rgb(AXIS_COLOR.0, AXIS_COLOR.1, AXIS_COLOR.2);

        cr.move_to(start_x, self.half_range_y);
        cr.line_to(self.last.x, self.half_range_y);
        cr.stroke();

        self.upper = upper;
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
    use gst_audio::AUDIO_FORMAT_S16;
    use log::info;

    use std::{
        env,
        fs::{create_dir, File},
        io::ErrorKind,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use crate::{AudioBuffer, AudioChannel, AudioChannelSide, SampleIndex, SampleIndexRange};
    use metadata::Duration;

    use super::*;

    const SAMPLE_RATE: u32 = 300;
    const SAMPLE_DURATION: Duration = Duration::from_frequency(SAMPLE_RATE as u64);
    const SAMPLE_WINDOW: SampleIndexRange = SampleIndexRange::new(300);
    const SAMPLE_DYN: i32 = 300;

    const CHANNELS: usize = 2;

    fn test_path() -> PathBuf {
        PathBuf::from(env!("OUT_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("test")
    }

    fn prepare_tests() {
        let test_path = test_path();
        let _ = create_dir(&test_path).map_err(|err| match err.kind() {
            ErrorKind::AlreadyExists => (),
            _ => panic!(
                "WaveformImage test: couldn't create directory {:?}",
                test_path
            ),
        });
    }

    fn init(sample_step_f: f64, width: i32) -> (AudioBuffer, WaveformImage, Dimensions) {
        //env_logger::init();
        gst::init().unwrap();

        prepare_tests();

        // AudioBuffer
        let mut audio_buffer = AudioBuffer::new(Duration::from_secs(1));
        audio_buffer.init(
            &gst_audio::AudioInfo::builder(AUDIO_FORMAT_S16, SAMPLE_RATE, CHANNELS as u32)
                .build()
                .unwrap(),
        );

        // WaveformImage
        let waveform = WaveformImage::new(
            0,
            Arc::new(Mutex::new(ChannelColors::default())),
            Arc::new(Mutex::new(None)),
        );
        let channels = vec![
            AudioChannel {
                side: AudioChannelSide::Left,
                factor: 1f64,
            },
            AudioChannel {
                side: AudioChannelSide::Right,
                factor: 1f64,
            },
        ];
        waveform.set_channels(channels.into_iter());

        let dimensions = Dimensions {
            sample_step: (sample_step_f as usize).max(1).into(),
            sample_step_f,

            x_step_f: 1f64,
            x_step: 1,

            req_width: width,
            req_width_f: width as f64,
            req_height: SAMPLE_DYN,

            ..Dimensions::default()
        };

        (audio_buffer, waveform, dimensions)
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
        let pts: gst::ClockTime = segment_lower.as_ts(SAMPLE_DURATION).into();
        {
            let buffer_mut = buffer.get_mut().unwrap();
            buffer_mut.set_pts(pts);
        }

        audio_buffer.have_gst_segment(&pts.into());
        audio_buffer.push_gst_buffer(&buffer, SampleIndex::default()); // never drain buffer in this test
    }

    fn render(
        prefix: &str,
        waveform: &mut WaveformImage,
        d: Dimensions,
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
        waveform.render(d, &audio_buffer, lower_to_extract, upper_to_extract);

        let lower = waveform.lower;
        let upper = waveform.upper;
        let image = waveform.image();

        let mut output_file = File::create(format!(
            "{}/waveform_image_{}_{}_{}.png",
            test_path().to_str().unwrap(),
            prefix,
            lower,
            upper
        ))
        .unwrap();
        image.with_surface(|surface| {
            surface.write_to_png(&mut output_file).unwrap();
        });
    }

    #[test]
    fn additive_draws() {
        let (mut audio_buffer, mut waveform, d) = init(3f64, 250);

        render(
            "additive_0 init",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(100, 200),
            SampleIndex::new(100),
        );
        render(
            "additive_1 overlap on the left and on the right",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(50, 250),
            SampleIndex::new(50),
        );
        render(
            "additive_2 overlap on the left",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(0, 100),
            SampleIndex::new(0),
        );
        render(
            "additive_3 scrolling and overlap on the right",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(150, 340),
            SampleIndex::new(150),
        );
        render(
            "additive_4 scrolling and overlaping on the right",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(0, 200),
            SampleIndex::new(250),
        );
    }

    #[test]
    fn link_between_draws() {
        let (mut audio_buffer, mut waveform, d) = init(1f64 / 5f64, 1480);

        render(
            "link_0",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(100, 200),
            SampleIndex::new(100),
        );
        // append to the left
        render(
            "link_1",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(25, 125),
            SampleIndex::new(0),
        );
        // appended to the right
        render(
            "link_2",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(175, 275),
            SampleIndex::new(200),
        );
    }

    #[test]
    fn seek() {
        let (mut audio_buffer, mut waveform, d) = init(1f64, 300);

        render(
            "seek_0",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(0, 100),
            SampleIndex::new(100),
        );
        // seeking forward
        render(
            "seek_1",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(0, 100),
            SampleIndex::new(500),
        );
        // additional samples
        render(
            "seek_2",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(100, 200),
            SampleIndex::new(600),
        );
        // additional samples
        render(
            "seek_3",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(200, 300),
            SampleIndex::new(700),
        );
    }

    #[test]
    fn oveflow() {
        let (mut audio_buffer, mut waveform, d) = init(1f64 / 5f64, 1500);

        render(
            "oveflow_0",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(0, 200),
            SampleIndex::new(250),
        );
        // overflow on the left
        render(
            "oveflow_1",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(0, 300),
            SampleIndex::new(0),
        );
        // overflow on the right
        render(
            "oveflow_2",
            &mut waveform,
            d.clone(),
            &mut audio_buffer,
            build_buffer(0, 100),
            SampleIndex::new(400),
        );
    }
}
