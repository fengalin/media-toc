use cairo;
use log::debug;
use smallvec::{smallvec, SmallVec};

use media::{
    AudioBuffer, AudioChannel, AudioChannelSide, SampleIndex, SampleIndexRange, INLINE_CHANNELS,
};

use super::Image;

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

#[derive(Default)]
pub struct WaveformSample {
    pub x: f64,
    pub y_values: SmallVec<[f64; INLINE_CHANNELS]>,
}

#[derive(Default)]
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
    sample_display_scale: f64,

    req_width: i32,
    req_height: i32,
    force_redraw: bool,

    pub lower: SampleIndex,
    pub upper: SampleIndex,

    pub contains_eos: bool,

    last: WaveformSample,

    pub sample_step_f: f64,
    pub sample_step: SampleIndexRange,
    pub x_step_f: f64,
    pub x_step: usize,
}

impl WaveformImage {
    pub fn new(id: usize) -> Self {
        WaveformImage {
            id,
            exposed_image: Some(
                Image::try_new(INIT_WIDTH, INIT_HEIGHT).expect("Default `WaveformImage`"),
            ),
            secondary_image: Some(
                Image::try_new(INIT_WIDTH, INIT_HEIGHT).expect("Default `WaveformImage`"),
            ),
            ..WaveformImage::default()
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
        self.sample_display_scale = 0f64;

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

        self.last = WaveformSample::default();

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

    pub fn update_width(&mut self, width: i32) -> Option<i32> {
        if width != self.req_width {
            self.force_redraw = true;
            self.shareable_state_changed = true;
            self.is_initialized = self.sample_step != SampleIndexRange::default();

            debug!(
                "{}_update_width prev. force_redraw {}, width {}, sample_step_f {}",
                self.id, self.force_redraw, self.req_width, self.sample_step_f
            );

            let prev_width = self.req_width;
            self.req_width = width;

            Some(prev_width)
        } else {
            None
        }
    }

    pub fn update_height(&mut self, height: i32) -> Option<i32> {
        if height != self.req_height {
            self.force_redraw = true;
            self.shareable_state_changed = true;

            debug!(
                "{}_update_height prev. force_redraw {}, height {}",
                self.id, self.force_redraw, self.req_height
            );

            let prev_height = self.req_height;
            self.req_height = height;

            Some(prev_height)
        } else {
            None
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
        } else if upper < self.lower || lower > self.upper || lower < self.lower {
            self.force_redraw = true;

            debug!(
                "{}_render forcing redraw image [{}, {}], requested [{}, {}] ",
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
                self.sample_display_scale = self.full_range_y / SAMPLE_RANGE;

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

        if !self.force_redraw {
            // append samples after previous last sample
            // shift previous image to the left (if necessary)
            // and append missing samples to the right
            self.append_right(exposed_image, secondary_image, audio_buffer, lower, upper);
        } else {
            self.redraw(exposed_image, secondary_image, audio_buffer, lower, upper);
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
        self.last.x = 0f64;
        self.last.y_values = smallvec![self.half_range_y; audio_buffer.channels];

        exposed_image.with_surface(|image_surface| {
            let cr = cairo::Context::new(&image_surface);

            cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
            cr.paint();

            self.draw_samples(&cr, audio_buffer, lower, upper);
        });

        debug!(
            "{}_redraw smpl_stp {}, lower {}, upper {}",
            self.id, self.sample_step, self.lower, self.upper
        );

        self.exposed_image = Some(exposed_image);
        self.secondary_image = Some(secondary_image);
        self.force_redraw = false;
        self.lower = lower;
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

        let x_range_to_draw = (upper - self.upper).get_step_range(self.sample_step) * self.x_step;
        let must_translate = self.last.x as usize + x_range_to_draw >= self.image_width as usize;

        if must_translate {
            // translate exposed image on secondary_image
            // secondary_image becomes the exposed image
            secondary_image.with_surface(|secondary_surface| {
                let cr = cairo::Context::new(&secondary_surface);

                exposed_image.with_surface_external_context(&cr, |cr, exposed_surface| {
                    cr.set_source_surface(exposed_surface, -x_offset, 0f64);
                    cr.paint();
                });

                self.lower = lower;

                self.last.x -= x_offset;
                self.clear_area(&cr, self.last.x, self.image_width_f);

                self.draw_samples(&cr, audio_buffer, self.upper, upper)
            });

            self.exposed_image = Some(secondary_image);
            self.secondary_image = Some(exposed_image);
        } else {
            // Don't translate => reuse exposed image
            exposed_image.with_surface(|exposed_surface| {
                let cr = cairo::Context::new(&exposed_surface);
                self.draw_samples(&cr, audio_buffer, self.upper, upper)
            });

            self.exposed_image = Some(exposed_image);
            self.secondary_image = Some(secondary_image);
        }
    }

    // Draw samples from sample_iter starting at first_x.
    fn draw_samples(
        &mut self,
        cr: &cairo::Context,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
    ) {
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
            cr.move_to(self.last.x + self.x_step_f, 0f64);
            cr.line_to(self.last.x + self.x_step_f, 0.5f64 * self.half_range_y);
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
            .try_iter(lower, upper, self.sample_step)
            .unwrap_or_else(|err| panic!("{}_draw_samples: {}", self.id, err));
        let start_x = self.last.x;

        for samples in samples_iter {
            let y_iter = samples.iter().map(|sample| {
                f64::from(i32::from(sample.as_i16()) - SAMPLE_AMPLITUDE) * sample_display_scale
            });

            let x = self.last.x + self.x_step_f;
            for (channel, y) in y_iter.enumerate() {
                let (r, g, b) = self
                    .channel_colors
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
    use gstreamer as gst;
    use gstreamer_audio as gst_audio;
    use gstreamer_audio::AUDIO_FORMAT_S16;
    use log::info;

    use std::fs::{create_dir, File};
    use std::io::ErrorKind;

    use media::{AudioBuffer, AudioChannel, AudioChannelSide, SampleIndex, SampleIndexRange};
    use metadata::Duration;

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
        waveform.update_width(width);
        waveform.update_height(SAMPLE_DYN);
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
