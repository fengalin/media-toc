#![feature(test)]

extern crate test;
use test::Bencher;

use byteorder::{ByteOrder, LittleEndian};

use gstreamer as gst;
use gstreamer_audio as gst_audio;

use media::{AudioBuffer, Duration, SampleIndex, Timestamp};

fn build_buffer(lower_value: usize, upper_value: usize, sample_duration: Duration) -> gst::Buffer {
    let lower: SampleIndex = lower_value.into();
    let pts = Timestamp::new(lower.get_ts(sample_duration).as_u64() + 1);
    let samples_u8_len = (upper_value - lower_value) * 2 * 2;

    let mut buffer = gst::Buffer::with_size(samples_u8_len).unwrap();
    {
        let buffer_mut = buffer.get_mut().unwrap();
        buffer_mut.set_pts(gst::ClockTime::from(pts.as_u64()));

        let mut buffer_map = buffer_mut.map_writable().unwrap();
        let buffer_slice = buffer_map.as_mut();

        let mut buf_u8 = [0; 2];
        for index in lower_value..upper_value {
            for channel in 0..2 {
                let value = if channel == 0 {
                    index as i16
                } else {
                    -(index as i16)
                };

                LittleEndian::write_i16(&mut buf_u8, value);
                let offset = (((index - lower_value) * 2) + channel) * 2;
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
fn bench_append_samples(b: &mut Bencher) {
    const SAMPLE_RATE: u32 = 48000;
    const SAMPLE_DURATION: Duration = Duration::from_frequency(SAMPLE_RATE as u64);

    const BUFFER_COUNT: usize = 1024;
    const SAMPLES_PER_BUFFER: usize = 1024;

    gst::init().unwrap();

    let mut audio_buffer = AudioBuffer::new(Duration::from_secs(10));
    audio_buffer.init(
        gst_audio::AudioInfo::new(gst_audio::AUDIO_FORMAT_S16, SAMPLE_RATE, 2)
            .build()
            .unwrap(),
    );

    let mut buffers = Vec::<(SampleIndex, gst::Buffer)>::with_capacity(BUFFER_COUNT);

    let mut lower = 0;
    let mut upper;
    for _ in 0..BUFFER_COUNT {
        upper = lower + SAMPLES_PER_BUFFER;
        buffers.push((lower.into(), build_buffer(lower, upper, SAMPLE_DURATION)));
        lower = upper;
    }

    b.iter(|| {
        audio_buffer.reset();
        audio_buffer.reset_segment_start();

        buffers.iter().for_each(|(lower, buffer)| {
            push_test_buffer(&mut audio_buffer, buffer, lower.as_usize() == 0);
        });
    });
}
