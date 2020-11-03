#![feature(test)]

extern crate test;
use test::Bencher;

use byteorder::{ByteOrder, LittleEndian};

use cairo;

use std::sync::{Arc, Mutex, RwLock};

use media::{
    sample_extractor, AudioBuffer, AudioChannel, AudioChannelSide, SampleExtractor, SampleIndex,
    SampleIndexRange, Timestamp,
};
use mediatocrenderers::{
    waveform::{image::ChannelColors, renderer::SharedState, Dimensions},
    WaveformRenderer,
};
use metadata::Duration;

const SAMPLE_RATE: u64 = 48_000;
const SAMPLE_DURATION: Duration = Duration::from_frequency(SAMPLE_RATE);

const CHANNELS: usize = 2;

const DURATION_FOR_1000: Duration = Duration::from_nanos(1_000_000_000_000u64 / SAMPLE_RATE);
const DURATION_FOR_1000PX: Duration = Duration::from_secs(4);

const BUFFER_COUNT: usize = 512;
const SAMPLES_PER_BUFFER: usize = 4096;

const BUFFER_OVERHEAD: usize = 5 * (SAMPLE_RATE as usize);

const DISPLAY_WIDTH: i32 = 1024;
const DISPLAY_HEIGHT: i32 = 500;

fn build_buffer(lower_value: usize, upper_value: usize) -> gst::Buffer {
    let lower: SampleIndex = lower_value.into();
    let pts = Timestamp::new(lower.as_ts(SAMPLE_DURATION).as_u64() + 1);
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

fn prepare_buffers() -> (AudioBuffer, WaveformRenderer, Arc<RwLock<SharedState>>) {
    let mut audio_buffer = AudioBuffer::new(Duration::from_secs(10));
    audio_buffer.init(
        &gst_audio::AudioInfo::builder(
            gst_audio::AUDIO_FORMAT_S16,
            SAMPLE_RATE as u32,
            CHANNELS as u32,
        )
        .build()
        .unwrap(),
    );

    push_test_buffer(
        &mut audio_buffer,
        &build_buffer(0, BUFFER_COUNT * SAMPLES_PER_BUFFER + BUFFER_OVERHEAD),
        true,
    );

    let shared_state = Arc::new(RwLock::new(SharedState::default()));

    (
        audio_buffer,
        WaveformRenderer::new(
            1,
            Arc::clone(&shared_state),
            Arc::new(RwLock::new(Dimensions::default())),
            Arc::new(RwLock::new(sample_extractor::State::default())),
            Arc::new(Mutex::new(ChannelColors::default())),
            Arc::new(Mutex::new(None)),
        ),
        shared_state,
    )
}

fn render_buffers(
    audio_buffer: &mut AudioBuffer,
    waveform_renderer: &mut WaveformRenderer,
    shared_state: &Arc<RwLock<SharedState>>,
    mut extra_op: Option<&mut dyn FnMut(usize, &mut WaveformRenderer)>,
) {
    // start with enough overhead in audio buffer
    audio_buffer.upper = BUFFER_OVERHEAD.into();

    waveform_renderer.reset();
    waveform_renderer.set_sample_duration(SAMPLE_DURATION, DURATION_FOR_1000);
    waveform_renderer.set_channels(
        vec![
            AudioChannel {
                side: AudioChannelSide::Left,
                factor: 1f64,
            },
            AudioChannel {
                side: AudioChannelSide::Right,
                factor: 1f64,
            },
        ]
        .into_iter(),
    );
    waveform_renderer.release();
    waveform_renderer.update_conditions(DURATION_FOR_1000PX, DISPLAY_WIDTH, DISPLAY_HEIGHT);

    for idx in 0..BUFFER_COUNT {
        let first_visible = idx * SAMPLES_PER_BUFFER;
        {
            let mut shared_state = shared_state.write().unwrap();
            shared_state.first_visible_sample = Some(first_visible.into());
            shared_state.cursor_sample = (first_visible + SAMPLES_PER_BUFFER / 2).into();
        }

        waveform_renderer.extract_samples(&audio_buffer);

        if let Some(extra_op) = extra_op.as_mut() {
            extra_op(idx, waveform_renderer);
        }

        if audio_buffer.upper.as_usize() < BUFFER_COUNT * SAMPLES_PER_BUFFER {
            audio_buffer.upper += SampleIndexRange::new(SAMPLES_PER_BUFFER);
        }
    }
}

#[bench]
fn bench_render_buffers(b: &mut Bencher) {
    gst::init().unwrap();

    let (mut audio_buffer, mut waveform_renderer, shared_state) = prepare_buffers();

    b.iter(|| {
        render_buffers(
            &mut audio_buffer,
            &mut waveform_renderer,
            &shared_state,
            None,
        );
    });
}

#[bench]
fn bench_render_buffers_and_display(b: &mut Bencher) {
    gst::init().unwrap();

    let (mut audio_buffer, mut waveform_renderer, shared_state) = prepare_buffers();

    let display_surface =
        cairo::ImageSurface::create(cairo::Format::Rgb24, DISPLAY_WIDTH, DISPLAY_HEIGHT)
            .expect("image surface");

    let mut render_to_display = |idx: usize, waveform_renderer: &mut WaveformRenderer| {
        let cr = cairo::Context::new(&display_surface);

        waveform_renderer
            .image
            .image()
            .with_surface_external_context(&cr, |cr, surface| {
                cr.set_source_surface(surface, -((idx % 20) as f64), 0f64);
                cr.paint();
            });
    };

    b.iter(|| {
        render_buffers(
            &mut audio_buffer,
            &mut waveform_renderer,
            &shared_state,
            Some(&mut render_to_display),
        );
    });
}
