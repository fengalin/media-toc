#![feature(test)]

extern crate test;
use test::Bencher;

use byteorder::{ByteOrder, LittleEndian};

use gstreamer as gst;
use gstreamer_audio as gst_audio;

use smallvec::SmallVec;

use media::{
    AudioBuffer, AudioChannel, Duration, SampleExtractor, SampleIndex, SampleIndexRange, Timestamp,
    INLINE_CHANNELS,
};
use renderers::WaveformBuffer;

const SAMPLE_RATE: u64 = 48000;
const SAMPLE_DURATION: Duration = Duration::from_frequency(SAMPLE_RATE);

const CHANNELS: usize = 2;

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

#[bench]
fn bench_render_buffers(b: &mut Bencher) {
    const DURATION_FOR_1000: Duration = Duration::from_nanos(1_000_000_000_000u64 / SAMPLE_RATE);
    const DURATION_FOR_1000PX: Duration = Duration::from_secs(4);

    const BUFFER_COUNT: usize = 512;
    const SAMPLES_PER_FRAME: usize = 1024;

    const BUFFER_OVERHEAD: usize = 2 * (SAMPLE_RATE as usize);

    gst::init().unwrap();

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

    let mut waveform_buffer = WaveformBuffer::new(1);

    b.iter(|| {
        // start with enough overhead in audio buffer
        audio_buffer.upper = BUFFER_OVERHEAD.into();

        waveform_buffer.reset();
        waveform_buffer.set_sample_duration(SAMPLE_DURATION, DURATION_FOR_1000);
        waveform_buffer.set_channels(&channels);
        waveform_buffer.set_state(gst::State::Playing);
        waveform_buffer.update_conditions(DURATION_FOR_1000PX, 1024, 768);

        for idx in 0..BUFFER_COUNT {
            let first_visible = idx * SAMPLES_PER_FRAME;
            waveform_buffer.first_visible_sample = Some(first_visible.into());
            waveform_buffer.cursor_sample = (first_visible + SAMPLES_PER_FRAME / 2).into();

            waveform_buffer.extract_samples(&audio_buffer);

            if audio_buffer.upper.as_usize() < BUFFER_COUNT * SAMPLES_PER_FRAME {
                audio_buffer.upper += SampleIndexRange::new(SAMPLES_PER_FRAME);
            }
        }
    });
}
