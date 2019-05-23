use cairo;
use log::{debug, warn};
use smallvec::SmallVec;

#[cfg(feature = "dump-waveform")]
use chrono::Utc;

#[cfg(feature = "dump-waveform")]
use std::{
    fs::{create_dir, File},
    io::ErrorKind,
};

use media::{
    AudioBuffer, AudioChannel, AudioChannelSide, SampleIndex, SampleIndexRange, INLINE_CHANNELS,
};

use super::Image;

pub const BACKGROUND_COLOR: (f64, f64, f64) = (0.2f64, 0.2235f64, 0.2314f64);
pub const AMPLITUDE_0_COLOR: (f64, f64, f64) = (0.5f64, 0.5f64, 0f64);

// Initial image dimensions
// will dynamically adapt if needed
const INIT_WIDTH: i32 = 1920;
const INIT_HEIGHT: i32 = 500;

#[cfg(feature = "dump-waveform")]
const WAVEFORM_DUMP_DIR: &str = "target/waveforms";

pub struct WaveformImage {
    pub id: usize,
    pub is_initialized: bool,
    pub is_ready: bool,
    pub shareable_state_changed: bool,

    channel_colors: SmallVec<[(f64, f64, f64); INLINE_CHANNELS]>,

    image: Option<Image>,
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

    pub last_x: Option<f64>,

    pub contains_eos: bool,

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

            image: Some(Image::try_new(INIT_WIDTH, INIT_HEIGHT).expect("Default `WaveformImage`")),
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

            last_x: None,

            sample_step_f: 0f64,
            sample_step: SampleIndexRange::default(),
            x_step_f: 0f64,
            x_step: 0,
        }
    }

    pub fn cleanup(&mut self) {
        // clear for reuse
        debug!("{}_cleanup", self.id);

        // self.image will be cleaned on next with draw
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

        self.last_x = None;

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
        self.image
            .as_mut()
            .expect("WaveformImage::get_image no image")
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
        if lower < audio_buffer.lower {
            // first sample might be smaller than audio_buffer.lower
            // due to alignement on sample_step
            lower += self.sample_step;
        }

        let upper = upper.get_aligned(self.sample_step);

        if lower >= upper {
            // can't draw current range
            // reset WaveformImage state
            debug!(
                "{}_render lower {} greater or equal upper {}",
                self.id, lower, upper
            );

            self.lower = SampleIndex::default();
            self.upper = SampleIndex::default();
            self.last_x = None;
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
        }

        let target_width = if self.image_width > 0 {
            self.image_width
                .max(((upper - lower).get_step_range(self.sample_step) * self.x_step) as i32)
        } else {
            INIT_WIDTH.max(((upper - lower).get_step_range(self.sample_step) * self.x_step) as i32)
        };

        let mut image = if target_width == self.image_width && self.req_height == self.image_height
        {
            self.image
                .take()
                .expect("WaveformImage::render image already taken")
        } else {
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

            Image::try_new(target_width, self.req_height).unwrap_or_else(|err| {
                panic!(
                    "WaveformBuffer.render creating {}x{} image: {}",
                    target_width, self.req_height, err,
                )
            })
        };

        let contains_eos = audio_buffer.contains_eos();

        self.last_x = image.with_surface(|image_surface| {
            let cr = cairo::Context::new(&image_surface);

            self.draw_samples(&cr, audio_buffer, lower, upper, contains_eos)
        });

        self.image = Some(image);

        if self.last_x.is_some() {
            self.lower = lower;
            self.upper = upper;
            self.contains_eos = contains_eos;
            self.force_redraw = false;
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

            self.image
                .as_mut()
                .expect("WaveformImage::render no image when dumping waveform")
                .with_surface(|surface| {
                    surface.write_to_png(&mut output_file).unwrap();
                });
        }

        self.is_ready = true;
    }

    #[allow(clippy::collapsible_if)]
    fn draw_samples(
        &self,
        cr: &cairo::Context,
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
        contains_eos: bool,
    ) -> Option<f64> {
        cr.set_source_rgb(BACKGROUND_COLOR.0, BACKGROUND_COLOR.1, BACKGROUND_COLOR.2);
        cr.paint();

        if self.x_step == 1 {
            cr.set_line_width(1f64);
        } else if self.x_step < 4 {
            cr.set_line_width(1.5f64);
        } else {
            cr.set_line_width(2f64);
        }

        let mut x = 0f64;
        for channel in 0..audio_buffer.channels {
            let mut y_iter = audio_buffer
                .try_iter(lower, upper, channel, self.sample_step)
                .unwrap_or_else(|err| panic!("{}_draw_samples: {}", self.id, err))
                .map(|channel_value| {
                    f64::from(i32::from(channel_value.as_i16()) - i32::from(std::i16::MAX))
                        * self.sample_value_factor
                });

            if channel == 0 {
                if y_iter.size_hint().0 < 2 && !contains_eos {
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
            }

            if let Some(&(red, green, blue)) = self.channel_colors.get(channel) {
                cr.set_source_rgba(red, green, blue, 0.68f64);
            } else {
                warn!("{}_draw_samples no color for channel {}", self.id, channel);
            }

            let y = y_iter
                .next()
                .unwrap_or_else(|| panic!("no first value for channel {}", channel));

            x = 0f64;
            cr.move_to(0f64, y);

            // draw the rest of the samples
            for y in y_iter {
                x += self.x_step_f;
                cr.line_to(x, y);
            }

            cr.stroke();
        }

        // Draw axis
        cr.set_line_width(1f64);
        cr.set_source_rgb(
            AMPLITUDE_0_COLOR.0,
            AMPLITUDE_0_COLOR.1,
            AMPLITUDE_0_COLOR.2,
        );

        cr.move_to(0f64, self.half_range_y);
        cr.line_to(x, self.half_range_y);
        cr.stroke();

        Some(x) // last x
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
