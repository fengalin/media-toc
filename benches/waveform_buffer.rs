#![feature(test)]

extern crate test;
use test::Bencher;

use byteorder::{ByteOrder, LittleEndian};

use cairo;

use gstreamer as gst;
use gstreamer_audio as gst_audio;

use smallvec::SmallVec;

use media::{
    AudioBuffer, AudioChannel, Duration, SampleExtractor, SampleIndex, SampleIndexRange, Timestamp,
    INLINE_CHANNELS,
};
use renderers::WaveformBuffer;

const SAMPLE_RATE: u64 = 48_000;
const SAMPLE_DURATION: Duration = Duration::from_frequency(SAMPLE_RATE);

const CHANNELS: usize = 2;

const DURATION_FOR_1000: Duration = Duration::from_nanos(1_000_000_000_000u64 / SAMPLE_RATE);
const DURATION_FOR_1000PX: Duration = Duration::from_secs(4);

const BUFFER_COUNT: usize = 512;
const SAMPLES_PER_FRAME: usize = 1024;

const BUFFER_OVERHEAD: usize = 2 * (SAMPLE_RATE as usize);

const DISPLAY_WIDTH: i32 = 800;
const DISPLAY_HEIGHT: i32 = 500;

const FONT_FAMILLY: &str = "Cantarell";
const FONT_SIZE: f64 = 15f64;
const TWICE_FONT_SIZE: f64 = 2f64 * FONT_SIZE;

const BOUNDARY_TEXT_MN: &str = "00:00.000";
const CURSOR_TEXT_MN: &str = "00:00.000.000";

fn build_buffer(lower_value: usize, upper_value: usize) -> gst::Buffer {
    let lower: SampleIndex = lower_value.into();
    let pts = Timestamp::new(lower.get_ts(SAMPLE_DURATION).as_u64() + 1);
    let samples_u8_len = (upper_value - lower_value) * CHANNELS * 2;

    let mut buffer = gst::Buffer::with_size(samples_u8_len).unwrap();
    {
        let buffer_mut = buffer.get_mut().unwrap();
        buffer_mut.set_pts(gst::ClockTime::from(pts.as_u64()));

        let mut buffer_map = buffer_mut.map_writable().unwrap();
        let buffer_slice = buffer_map.as_mut();

        let mut buf_u8 = [0; CHANNELS];
        for index in lower_value..upper_value {
            for channel in 0..CHANNELS {
                let value = if channel == 0 {
                    index as i16
                } else {
                    -(index as i16)
                };

                LittleEndian::write_i16(&mut buf_u8, value);
                let offset = (((index - lower_value) * CHANNELS) + channel) * 2;
                buffer_slice[offset] = buf_u8[0];
                buffer_slice[offset + 1] = buf_u8[1];
            }
        }
    }

    buffer
}

fn push_test_buffer(audio_buffer: &mut AudioBuffer, buffer: &gst::Buffer, is_new_segment: bool) {
    if is_new_segment {
        audio_buffer.have_gst_segment(buffer.get_pts().nseconds().unwrap().into());
    }

    audio_buffer.push_gst_buffer(buffer, SampleIndex::default()); // never drain buffer in this test
}

fn prepare_buffers() -> (
    AudioBuffer,
    WaveformBuffer,
    SmallVec<[AudioChannel; INLINE_CHANNELS]>,
) {
    let mut audio_buffer = AudioBuffer::new(Duration::from_secs(10));
    audio_buffer.init(
        gst_audio::AudioInfo::new(
            gst_audio::AUDIO_FORMAT_S16,
            SAMPLE_RATE as u32,
            CHANNELS as u32,
        )
        .build()
        .unwrap(),
    );

    push_test_buffer(
        &mut audio_buffer,
        &build_buffer(0, BUFFER_COUNT * SAMPLES_PER_FRAME + BUFFER_OVERHEAD),
        true,
    );

    let mut channels: SmallVec<[AudioChannel; INLINE_CHANNELS]> = SmallVec::with_capacity(CHANNELS);
    channels.push(AudioChannel::new(
        gst_audio::AudioChannelPosition::FrontLeft,
    ));
    channels.push(AudioChannel::new(
        gst_audio::AudioChannelPosition::FrontRight,
    ));

    (audio_buffer, WaveformBuffer::new(1), channels)
}

fn render_buffers(
    audio_buffer: &mut AudioBuffer,
    waveform_buffer: &mut WaveformBuffer,
    channels: &SmallVec<[AudioChannel; INLINE_CHANNELS]>,
    mut extra_op: Option<&mut FnMut(usize, &mut WaveformBuffer)>,
) {
    // start with enough overhead in audio buffer
    audio_buffer.upper = BUFFER_OVERHEAD.into();

    waveform_buffer.reset();
    waveform_buffer.set_sample_duration(SAMPLE_DURATION, DURATION_FOR_1000);
    waveform_buffer.set_channels(&channels);
    waveform_buffer.set_state(gst::State::Playing);
    waveform_buffer.update_conditions(DURATION_FOR_1000PX, DISPLAY_WIDTH, DISPLAY_HEIGHT);

    for idx in 0..BUFFER_COUNT {
        let first_visible = idx * SAMPLES_PER_FRAME;
        waveform_buffer.first_visible_sample = Some(first_visible.into());
        waveform_buffer.cursor_sample = (first_visible + SAMPLES_PER_FRAME / 2).into();

        waveform_buffer.extract_samples(&audio_buffer);

        if let Some(extra_op) = extra_op.as_mut() {
            extra_op(idx, waveform_buffer);
        }

        if audio_buffer.upper.as_usize() < BUFFER_COUNT * SAMPLES_PER_FRAME {
            audio_buffer.upper += SampleIndexRange::new(SAMPLES_PER_FRAME);
        }
    }
}

#[bench]
fn bench_render_buffers(b: &mut Bencher) {
    gst::init().unwrap();

    let (mut audio_buffer, mut waveform_buffer, channels) = prepare_buffers();

    b.iter(|| {
        render_buffers(&mut audio_buffer, &mut waveform_buffer, &channels, None);
    });
}

#[bench]
fn bench_render_buffers_and_display(b: &mut Bencher) {
    gst::init().unwrap();

    let (mut audio_buffer, mut waveform_buffer, channels) = prepare_buffers();

    let display_surface =
        cairo::ImageSurface::create(cairo::Format::Rgb24, DISPLAY_WIDTH, DISPLAY_HEIGHT)
            .expect("image surface");

    let mut render_to_display = |idx: usize, waveform_buffer: &mut WaveformBuffer| {
        let cr = cairo::Context::new(&display_surface);

        waveform_buffer
            .image
            .get_image()
            .with_surface_external_context(&cr, |cr, surface| {
                cr.set_source_surface(surface, -((idx % 20) as f64), 0f64);
                cr.paint();
            });

        // Draw the cursor in the middle
        let middle_x = (DISPLAY_WIDTH / 2) as f64;
        cr.set_source_rgb(1f64, 1f64, 0f64);
        cr.set_line_width(1f64);
        cr.move_to(middle_x, 0f64);
        cr.line_to(middle_x, TWICE_FONT_SIZE);
        cr.stroke();

        // Add text at cursor and boundaries
        cr.select_font_face(
            FONT_FAMILLY,
            cairo::FontSlant::Normal,
            cairo::FontWeight::Normal,
        );
        cr.set_font_size(FONT_SIZE);

        cr.move_to(middle_x, TWICE_FONT_SIZE);
        cr.show_text(CURSOR_TEXT_MN);

        cr.move_to(2f64, TWICE_FONT_SIZE);
        cr.show_text(BOUNDARY_TEXT_MN);

        cr.move_to(middle_x + 0.5f64 * middle_x, TWICE_FONT_SIZE);
        cr.show_text(BOUNDARY_TEXT_MN);
    };

    b.iter(|| {
        render_buffers(
            &mut audio_buffer,
            &mut waveform_buffer,
            &channels,
            Some(&mut render_to_display),
        );
    });
}
