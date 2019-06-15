use cairo;
use log::{debug, warn};
use smallvec::SmallVec;

use std::iter::Iterator;

use media::{AudioChannel, AudioChannelSide, INLINE_CHANNELS};

use super::WaveformBuffer;

pub const AMPLITUDE_0_COLOR: (f64, f64, f64) = (0.5f64, 0.5f64, 0f64);

#[derive(Default)]
pub struct WaveformTracer {
    id: usize,

    is_initialized: bool,

    channel_colors: SmallVec<[(f64, f64, f64); INLINE_CHANNELS]>,

    pub(super) width: i32,
    pub(super) width_f: f64,
    height: i32,
    height_f: f64,
    pub(super) half_range_y: f64,

    pub(super) x_step_f: f64,
    x_step: usize,
}

impl WaveformTracer {
    pub fn new(id: usize) -> Self {
        WaveformTracer {
            id,
            ..WaveformTracer::default()
        }
    }

    pub fn reset(&mut self) {
        // clear for reuse
        debug!("{}_reset", self.id);

        // self.image will be cleaned on next with draw
        self.is_initialized = false;
        self.width = 0;
        self.width_f = 0f64;
        self.height = 0;
        self.height_f = 0f64;
        self.half_range_y = 0f64;

        self.reset_conditions();
    }

    pub fn reset_conditions(&mut self) {
        debug!("{}_reset_conditions", self.id);

        self.channel_colors.clear();

        self.x_step_f = 0f64;
        self.x_step = 0;
    }

    pub fn set_channels(&mut self, channels: &[AudioChannel]) {
        debug!("{}_set_channels {}", self.id, channels.len());

        for channel in channels.iter().take(INLINE_CHANNELS) {
            self.channel_colors.push(match channel.side {
                AudioChannelSide::Center => (0f64, channel.factor, 0f64),
                AudioChannelSide::Left => (channel.factor, channel.factor, channel.factor),
                AudioChannelSide::NotLocalized => (0f64, 0f64, channel.factor),
                AudioChannelSide::Right => (channel.factor, 0f64, 0f64),
            });
        }
    }

    // Return previous width if width has changed
    pub fn update_width(&mut self, width: i32) -> Option<i32> {
        if width != self.width {
            debug!("{}_update_width {} -> {}", self.id, self.width, width);

            let previous_width = self.width;

            self.width = width;
            self.width_f = f64::from(width);

            self.is_initialized = (self.x_step != 0) && (self.width != 0);

            Some(previous_width)
        } else {
            None
        }
    }

    // Return previous height if height has changed
    pub fn update_height(&mut self, height: i32) -> Option<i32> {
        if height != self.height {
            debug!("{}_update_height {} -> {}", self.id, self.height, height);

            let previous_height = self.height;

            self.height = height;
            self.height_f = f64::from(height);
            self.half_range_y = self.height_f / 2f64;

            Some(previous_height)
        } else {
            None
        }
    }

    pub fn update_x_step(&mut self, sample_step_f: f64) {
        self.x_step_f = if sample_step_f < 1f64 {
            (1f64 / sample_step_f).round()
        } else {
            1f64
        };

        self.x_step = self.x_step_f as usize;

        self.is_initialized = (self.x_step != 0) && (self.width != 0);
    }

    pub fn update_from_other(&mut self, other: &mut WaveformTracer) {
        debug!("{}_update_from_other", self.id);

        self.width = other.width;
        self.width_f = other.width_f;
        self.height = other.height;
        self.height_f = other.height_f;
        self.half_range_y = other.half_range_y;
        self.x_step = other.x_step;
        self.x_step_f = other.x_step_f;
        self.is_initialized = other.is_initialized;
    }

    pub fn draw(
        &self,
        cr: &cairo::Context,
        buffer: &WaveformBuffer,
        first_index: usize,
        last_index: usize,
        last_x: f64,
    ) {
        // Draw axis
        cr.set_line_width(1f64);
        cr.set_source_rgb(
            AMPLITUDE_0_COLOR.0,
            AMPLITUDE_0_COLOR.1,
            AMPLITUDE_0_COLOR.2,
        );

        cr.move_to(0f64, self.half_range_y);
        cr.line_to(last_x, self.half_range_y);
        cr.stroke();

        // Draw waveform
        if self.x_step == 1 {
            cr.set_line_width(1f64);
        } else if self.x_step < 4 {
            cr.set_line_width(1.5f64);
        } else {
            cr.set_line_width(2f64);
        }

        for (channel_idx, samples) in buffer.iter().enumerate() {
            if let Some(&(red, green, blue)) = self.channel_colors.get(channel_idx) {
                cr.set_source_rgb(red, green, blue);
            } else {
                warn!(
                    "{}_draw_samples no color for channel {}",
                    self.id, channel_idx
                );
            }

            let mut x = 0f64;
            cr.move_to(0f64, samples[first_index]);

            for y in samples[first_index + 1..last_index].iter() {
                x += self.x_step_f;
                cr.line_to(x, *y);
            }

            cr.stroke();
        }
    }
}
