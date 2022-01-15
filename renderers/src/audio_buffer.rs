use byteorder::{BigEndian, LittleEndian, ReadBytesExt};

use gst_audio::{prelude::*, AudioFormat};

use log::{debug, trace};

use dasp_sample::Sample;

use std::{
    collections::vec_deque::VecDeque,
    io::{Cursor, Read},
};

use metadata::Duration;

use super::{SampleIndex, SampleIndexRange, SampleValue, Timestamp, INLINE_CHANNELS};

#[derive(Debug)]
pub struct StreamState {
    format: gst_audio::AudioFormat,
    rate: u64,
    bytes_per_channel: usize,
    sample_duration: Duration,
    channels: usize,

    eos: bool,

    is_new_segment: bool,
    segment_start: Option<Timestamp>,
    segment_lower: SampleIndex,

    last_buffer_lower: SampleIndex,
    last_buffer_upper: SampleIndex,
}

impl StreamState {
    fn new() -> Self {
        StreamState {
            format: gst_audio::AudioFormat::Unknown,
            rate: 0,
            channels: 0,
            bytes_per_channel: 0,
            sample_duration: Duration::default(),

            eos: false,

            is_new_segment: true,
            segment_start: None,
            segment_lower: SampleIndex::default(),
            last_buffer_lower: SampleIndex::default(),
            last_buffer_upper: SampleIndex::default(),
        }
    }

    fn init(&mut self, audio_info: &gst_audio::AudioInfo) {
        // assert_eq!(layout, Interleaved);
        self.format = audio_info.format();
        self.rate = u64::from(audio_info.rate());
        self.channels = audio_info.channels() as usize;
        self.bytes_per_channel = audio_info.width() as usize / 8;
        self.sample_duration = Duration::from_frequency(self.rate);

        self.segment_lower = SampleIndex::default();
        self.last_buffer_lower = SampleIndex::default();
        self.last_buffer_upper = SampleIndex::default();
    }

    fn cleanup(&mut self) {
        self.reset();
        self.segment_start = None;
    }

    fn forget_past(&mut self) {
        self.eos = false;
        self.is_new_segment = true;
        // don't cleanup self.segment_start in order to maintain continuity
        self.segment_lower = SampleIndex::default();
        self.last_buffer_lower = SampleIndex::default();
        self.last_buffer_upper = SampleIndex::default();
    }

    fn reset(&mut self) {
        self.format = gst_audio::AudioFormat::Unknown;
        self.rate = 0;
        self.channels = 0;
        self.bytes_per_channel = 0;
        self.sample_duration = Duration::default();
    }

    fn set_eos(&mut self) {
        self.eos = true;
    }

    fn clear_eos(&mut self) {
        self.eos = false;
    }

    fn have_segment(&mut self, segment: &gst::FormattedSegment<gst::ClockTime>) {
        // FIXME use ClockTime everywhere
        let gst_time = segment.time();
        let time = Timestamp::try_from(gst_time).unwrap();

        debug!(
            "have_gst_segment {} ({})",
            gst_time.display(),
            time.sample_index(self.sample_duration),
        );

        match self.segment_start {
            Some(cur_segment_start) => {
                if cur_segment_start != time {
                    self.is_new_segment = true;
                }
                // else: same segment => might be an async_done after a pause
                //       or a seek back to the segment's start
            }
            None => self.is_new_segment = true,
        }

        if self.is_new_segment {
            self.segment_start = Some(time);
            self.segment_lower = SampleIndex::from_ts(time, self.sample_duration);
            self.last_buffer_upper = self.segment_lower;
            self.is_new_segment = false;
        }
    }

    fn have_buffer(&mut self, buffer: &gst::Buffer) -> SampleIndexRange {
        self.last_buffer_lower = self.last_buffer_upper;
        let incoming_range =
            SampleIndexRange::new(buffer.size() / self.channels / self.bytes_per_channel);
        self.last_buffer_upper += incoming_range;

        incoming_range
    }
}

#[derive(Debug)]
pub struct AudioBuffer {
    pub(super) buffer_duration: Duration,
    capacity: usize,
    stream_state: StreamState,
    // AudioBuffer stores up to INLINE_CHANNELS
    pub channels: usize,
    drain_size: usize,

    pub lower: SampleIndex,
    pub upper: SampleIndex,
    // FIXME we probably no longer need a VecDeque
    pub samples: VecDeque<SampleValue>,
}

impl AudioBuffer {
    pub fn new(buffer_duration: Duration) -> Self {
        AudioBuffer {
            buffer_duration,
            capacity: 0,
            stream_state: StreamState::new(),
            channels: 0,
            drain_size: 0,

            lower: SampleIndex::default(),
            upper: SampleIndex::default(),
            samples: VecDeque::new(),
        }
    }

    // FIXME: find more explicite names and rationalize `init`, `cleanup`, `reset`, ...
    pub fn init(&mut self, audio_info: &gst_audio::AudioInfo) {
        // assert_eq!(layout, Interleaved);

        // changing caps
        self.cleanup();

        self.stream_state.init(audio_info);
        self.channels = INLINE_CHANNELS.min(self.stream_state.channels);
        self.capacity = SampleIndexRange::from_duration(
            self.buffer_duration,
            self.stream_state.sample_duration,
        )
        .as_usize()
            * self.channels;
        self.samples = VecDeque::with_capacity(self.capacity);
        self.drain_size = self.stream_state.rate as usize * self.channels; // 1s worth of samples

        debug!(
            "init rate {}, channels {}",
            self.stream_state.rate, self.channels
        );
    }

    // Clean everything so that the AudioBuffer
    // can be reused for a different media
    pub fn cleanup(&mut self) {
        debug!("cleaning up");

        self.reset();
        self.stream_state.cleanup();
    }

    // Clean the sample buffer
    // Other characteristics (rate, sample_duration, channels) remain unchanged.
    pub fn clean_samples(&mut self) {
        debug!("clean_samples");
        self.stream_state.forget_past();
        self.lower = SampleIndex::default();
        self.upper = SampleIndex::default();
        self.samples.clear();
    }

    // Reset the AudioBuffer keeping continuity
    // This is required in case of a caps change or stream change
    // as samples may come in the same segment despite the change.
    // If the media is paused and then set back to playback, preroll
    // will be performed in the same segment as before the change.
    // So we need to keep track of the segment start in order not
    // to reset current sequence (self.stream_state.last_buffer_upper).
    pub fn reset(&mut self) {
        debug!("resetting");

        self.stream_state.reset();
        self.capacity = 0;
        self.channels = 0;
        self.drain_size = 0;
        self.clean_samples();
    }

    pub fn reset_segment_start(&mut self) {
        self.stream_state.segment_start = None;
    }

    pub fn have_gst_segment(&mut self, segment: &gst::FormattedSegment<gst::ClockTime>) {
        self.stream_state.have_segment(segment);
    }

    // Add samples from the GStreamer pipeline to the AudioBuffer
    // This buffer stores the complete set of samples in a time frame
    // in order to be able to represent the audio at any given precision.
    // Incoming samples are merged to the existing buffer when possible
    // Returns: number of samples received
    pub fn push_gst_buffer(
        &mut self,
        gst_buffer: &gst::Buffer,
        lower_to_keep: SampleIndex,
    ) -> SampleIndexRange {
        if self.stream_state.channels == 0 {
            debug!("push_gst_buffer: audio characterists not defined yet");
            return SampleIndexRange::default();
        }

        let incoming_range = self.stream_state.have_buffer(gst_buffer);
        let incoming_lower = self.stream_state.last_buffer_lower;
        let incoming_upper = self.stream_state.last_buffer_upper;

        struct ProcessingInstructions {
            append_after: bool,
            lower_to_add_rel: SampleIndex,
            upper_to_add_rel: SampleIndex,
        }

        // Identify conditions for this incoming gst_buffer:
        // 1. Incoming gst_buffer fits at the end of current container.
        // 2. Incoming gst_buffer is already contained within stored samples.
        //    Nothing to do.
        // 3. Incoming gst_buffer overlaps with stored samples at the end.
        // 4. Incoming gst_buffer overlaps with stored samples at the begining.
        //    Note: this changes the lower sample and requires to extend
        //    the internal container from the begining.
        // 5. Incoming gst_buffer doesn't overlap with current buffer. In order
        //    not to let gaps between samples, the internal container is
        //    cleared lower.
        // 6. The internal container is empty, import incoming gst_buffer
        //    completely.
        let ins = if !self.samples.is_empty() {
            // not initializing
            if incoming_lower == self.upper {
                // 1. append incoming gst_buffer to the end of internal storage
                trace!("case 1. appending to the end (full)");
                // self.lower unchanged
                self.upper = incoming_upper;
                self.stream_state.clear_eos();

                ProcessingInstructions {
                    append_after: true,
                    lower_to_add_rel: SampleIndex::default(),
                    upper_to_add_rel: incoming_range.into(),
                }
            } else if incoming_lower >= self.lower && incoming_upper <= self.upper {
                // 2. incoming gst_buffer included in current container
                debug!(
                    concat!(
                        "case 2. contained in current container ",
                        "self [{}, {}], incoming [{}, {}]",
                    ),
                    self.lower, self.upper, incoming_lower, incoming_upper
                );

                ProcessingInstructions {
                    append_after: false,
                    lower_to_add_rel: SampleIndex::default(),
                    upper_to_add_rel: SampleIndex::default(),
                }
            } else if incoming_lower > self.lower && incoming_lower < self.upper {
                // 3. can append [self.upper, upper] to the end
                debug!(
                    "case 3. append to the end (partial) [{}, {}], incoming [{}, {}]",
                    self.lower, self.upper, incoming_lower, incoming_upper
                );
                // self.lower unchanged
                let previous_upper = self.upper;
                self.upper = incoming_upper;
                self.stream_state.clear_eos();

                // self.first_pts unchanged
                ProcessingInstructions {
                    append_after: true,
                    lower_to_add_rel: (previous_upper - incoming_lower).into(),
                    upper_to_add_rel: incoming_range.into(),
                }
            } else if incoming_upper < self.upper && incoming_upper >= self.lower {
                // 4. can insert [lower, self.lower] at the begining
                debug!(
                    "case 4. insert at the begining [{}, {}], incoming [{}, {}]",
                    self.lower, self.upper, incoming_lower, incoming_upper
                );
                let upper_to_add = self.lower;
                self.lower = incoming_lower;
                // self.upper unchanged

                ProcessingInstructions {
                    append_after: false,
                    lower_to_add_rel: SampleIndex::default(),
                    upper_to_add_rel: (upper_to_add - incoming_lower).into(),
                }
            } else {
                // 5. can't merge with previous gst_buffer
                debug!(
                    "case 5. can't merge self [{}, {}], incoming [{}, {}]",
                    self.lower, self.upper, incoming_lower, incoming_upper
                );
                self.samples.clear();
                self.lower = incoming_lower;
                self.upper = incoming_upper;
                self.stream_state.clear_eos();

                ProcessingInstructions {
                    append_after: true,
                    lower_to_add_rel: SampleIndex::default(),
                    upper_to_add_rel: incoming_range.into(),
                }
            }
        } else {
            // 6. initializing
            debug!("init [{}, {}]", incoming_lower, incoming_upper);
            self.lower = incoming_lower;
            self.upper = incoming_upper;
            self.stream_state.clear_eos();

            ProcessingInstructions {
                append_after: true,
                lower_to_add_rel: SampleIndex::default(),
                upper_to_add_rel: incoming_range.into(),
            }
        };

        // Don't drain if samples are to be added at the begining...
        // drain only if we have enough samples in history
        // TODO: it could be worth testing truncate instead
        // (this would require reversing the buffer alimentation
        // and iteration).
        // Don't drain samples if they might be used by the extractor
        // (limit known as argument lower_to_keep).
        if ins.append_after
            && self.samples.len()
                + (ins.upper_to_add_rel - ins.lower_to_add_rel).as_usize() * self.channels
                > self.capacity
            && lower_to_keep.min(incoming_lower)
                > self.lower + SampleIndexRange::new(self.drain_size / self.channels)
        {
            debug!("draining... len before: {}", self.samples.len());
            self.samples.drain(..self.drain_size);
            self.lower += SampleIndexRange::new(self.drain_size / self.channels);
        }

        if ins.upper_to_add_rel > SampleIndex::default() {
            let buffer = gst_buffer.map_readable().unwrap();
            let converter_iter = SampleConverterIter::from_slice(
                buffer.as_slice(),
                self,
                ins.lower_to_add_rel,
                ins.upper_to_add_rel,
            )
            .unwrap();

            if ins.append_after {
                for sample in converter_iter {
                    self.samples.push_back(sample);
                }
            } else {
                for sample in converter_iter.rev() {
                    self.samples.push_front(sample);
                }
            }
        }

        incoming_range
    }

    pub fn segment_lower(&self) -> SampleIndex {
        self.stream_state.segment_lower
    }

    pub fn contains_eos(&self) -> bool {
        self.stream_state.eos
    }

    pub fn handle_eos(&mut self) {
        // EOS can be received when seeking within a range
        // in our case, this occurs in paused mode at the end of a range playback.
        // In this situation, the last samples received should already be contained
        // in the AudioBuffer.
        if !self.samples.is_empty() {
            self.stream_state.set_eos();
        }
        self.stream_state.segment_start = None;
    }

    pub fn try_iter(
        &self,
        lower: SampleIndex,
        upper: SampleIndex,
        sample_step: SampleIndexRange,
    ) -> Result<Iter<'_>, String> {
        Iter::try_new(self, lower, upper, sample_step)
    }

    pub fn get(&self, sample_idx: SampleIndex) -> Option<&[SampleValue]> {
        if sample_idx >= self.lower && sample_idx < self.upper {
            let slices = self.samples.as_slices();
            let slice0_len = slices.0.len();
            let mut idx = (sample_idx - self.lower).as_usize() * self.channels;
            let last_idx = idx + self.channels;

            if last_idx <= slice0_len {
                Some(&slices.0[idx..last_idx])
            } else if last_idx <= self.samples.len() {
                idx -= slice0_len;
                Some(&slices.1[idx..idx + self.channels])
            } else {
                None
            }
        } else {
            None
        }
    }
}

// Convert sample buffer to SampleValue on the fly
type ConvertFn = fn(&mut dyn Read) -> SampleValue;
macro_rules! to_sample_value(
    ($read:expr) => {
        SampleValue::from($read.unwrap().to_sample::<i16>())
    }
);
pub struct SampleConverterIter<'slice> {
    cursor: Cursor<&'slice [u8]>,
    sample_step: u64,
    bytes_per_channel: u64,
    two_x_bytes_per_channel: u64,
    output_channels: usize,
    extra_positions: u64,
    convert: ConvertFn,
    first: SampleIndex,
    idx: Option<(SampleIndex, usize)>,
    last: SampleIndex,
}

impl<'slice> SampleConverterIter<'slice> {
    fn from_slice(
        slice: &'slice [u8],
        audio_buffer: &AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
    ) -> Option<SampleConverterIter<'slice>> {
        let mut cursor = Cursor::new(slice);

        let bytes_per_channel = audio_buffer.stream_state.bytes_per_channel;
        let stream_channels = audio_buffer.stream_state.channels;
        let sample_step = (stream_channels * bytes_per_channel) as u64;
        cursor.set_position(lower.as_u64() * sample_step);

        let output_channels = audio_buffer.channels;
        let extra_positions = if output_channels < stream_channels {
            sample_step - (output_channels * bytes_per_channel) as u64
        } else {
            0u64
        };

        let bytes_per_channel = audio_buffer.stream_state.bytes_per_channel as u64;

        Some(SampleConverterIter {
            cursor,
            sample_step,
            bytes_per_channel,
            two_x_bytes_per_channel: 2 * bytes_per_channel,
            output_channels,
            extra_positions,
            convert: SampleConverterIter::convert_fn(audio_buffer.stream_state.format),
            first: lower,
            idx: None,
            last: upper,
        })
    }

    fn convert_fn(format: gst_audio::AudioFormat) -> ConvertFn {
        let convert: ConvertFn = match format {
            AudioFormat::S8 => |rdr| to_sample_value!(rdr.read_i8()),
            AudioFormat::U8 => |rdr| to_sample_value!(rdr.read_u8()),
            AudioFormat::S16le => |rdr| to_sample_value!(rdr.read_i16::<LittleEndian>()),
            AudioFormat::S16be => |rdr| to_sample_value!(rdr.read_i16::<BigEndian>()),
            AudioFormat::U16le => |rdr| to_sample_value!(rdr.read_u16::<LittleEndian>()),
            AudioFormat::U16be => |rdr| to_sample_value!(rdr.read_u16::<BigEndian>()),
            AudioFormat::S32le => |rdr| to_sample_value!(rdr.read_i32::<LittleEndian>()),
            AudioFormat::S32be => |rdr| to_sample_value!(rdr.read_i32::<BigEndian>()),
            AudioFormat::U32le => |rdr| to_sample_value!(rdr.read_u32::<LittleEndian>()),
            AudioFormat::U32be => |rdr| to_sample_value!(rdr.read_u32::<BigEndian>()),
            AudioFormat::F32le => |rdr| to_sample_value!(rdr.read_f32::<LittleEndian>()),
            AudioFormat::F32be => |rdr| to_sample_value!(rdr.read_f32::<BigEndian>()),
            AudioFormat::F64le => |rdr| to_sample_value!(rdr.read_f64::<LittleEndian>()),
            AudioFormat::F64be => |rdr| to_sample_value!(rdr.read_f64::<BigEndian>()),
            _ => unimplemented!("Converting to {:?}", format),
        };

        convert
    }
}

impl<'slice> Iterator for SampleConverterIter<'slice> {
    type Item = SampleValue;

    fn next(&mut self) -> Option<Self::Item> {
        match self.idx {
            Some((idx, _)) => {
                if idx >= self.last {
                    return None;
                }
            }
            None => self.idx = Some((self.first, 0)),
        }

        //assert!(self.idx.is_some());

        let channel_value = (self.convert)(&mut self.cursor);

        let (idx, channel) = self.idx.as_mut().unwrap();
        *channel += 1;
        if *channel == self.output_channels {
            *channel = 0;
            idx.inc();
            self.cursor
                .set_position(self.cursor.position() + self.extra_positions);
        }

        Some(channel_value)
    }
}

impl<'slice> DoubleEndedIterator for SampleConverterIter<'slice> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match &mut self.idx {
            Some((idx, channel)) => {
                if (idx == &self.first) && (*channel == 0) {
                    return None;
                } else if *channel > 0 {
                    *channel -= 1;
                    // get back 2x bytes_per_channel positions:
                    // 1x for the bytes previously read
                    // 1x for the bytes to read
                    self.cursor
                        .set_position(self.cursor.position() - self.two_x_bytes_per_channel);
                } else {
                    *channel = self.output_channels - 1;
                    idx.try_dec().ok()?;
                    // get back:
                    // 1x for the bytes previously read
                    // 1x for the bytes to read
                    // skip the extra positions
                    self.cursor.set_position(
                        self.cursor.position()
                            - self.two_x_bytes_per_channel
                            - self.extra_positions,
                    );
                }
            }
            None => {
                let channel = self.output_channels - 1;
                let mut idx = self.last;
                idx.try_dec().ok()?;
                self.idx = Some((idx, channel));
                self.cursor.set_position(
                    idx.as_u64() * self.sample_step + (channel as u64 * self.bytes_per_channel),
                );
            }
        }

        Some((self.convert)(&mut self.cursor))
    }
}

pub struct Iter<'buffer> {
    slice0: &'buffer [SampleValue],
    slice0_len: usize,
    slice1: &'buffer [SampleValue],
    channels: usize,
    idx: usize,
    upper: usize,
    step: usize,
}

impl<'buffer> Iter<'buffer> {
    fn try_new(
        audio_buffer: &'buffer AudioBuffer,
        lower: SampleIndex,
        upper: SampleIndex,
        sample_step: SampleIndexRange,
    ) -> Result<Iter<'buffer>, String> {
        if upper > lower && lower >= audio_buffer.lower && upper <= audio_buffer.upper {
            let slices = audio_buffer.samples.as_slices();
            let len0 = slices.0.len();
            Ok(Iter {
                slice0: slices.0,
                slice0_len: len0,
                slice1: slices.1,
                channels: audio_buffer.channels,
                idx: (lower - audio_buffer.lower).as_usize() * audio_buffer.channels,
                upper: (upper - audio_buffer.lower).as_usize() * audio_buffer.channels,
                step: sample_step.as_usize() * audio_buffer.channels,
            })
        } else {
            Err(format!(
                "Iter::try_new [{}, {}] out of bounds [{}, {}]",
                lower, upper, audio_buffer.lower, audio_buffer.upper,
            ))
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }
}

impl<'buffer> Iterator for Iter<'buffer> {
    type Item = &'buffer [SampleValue];

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.upper {
            return None;
        }

        let idx = self.idx;
        let item = if idx < self.slice0_len {
            &self.slice0[idx..idx + self.channels]
        } else {
            let idx = idx - self.slice0_len;
            &self.slice1[idx..idx + self.channels]
        };

        self.idx += self.step;
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.idx >= self.upper {
            return (0, Some(0));
        }

        let remaining = (self.upper - self.idx) / self.step;

        (remaining, Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    //use env_logger;

    use byteorder::{ByteOrder, LittleEndian};
    use gst_audio::AUDIO_FORMAT_S16;
    use log::{debug, info};

    use crate::{SampleIndex, SampleIndexRange, SampleValue, Timestamp};
    use metadata::Duration;

    use super::AudioBuffer;

    const SAMPLE_RATE: u32 = 300;
    const SAMPLE_DURATION: Duration = Duration::from_frequency(SAMPLE_RATE as u64);
    const CHANNELS: usize = 2;

    // Build a buffer with 2 channels in the specified range
    // which would be rendered as a diagonal on a Waveform image
    // from left top corner to right bottom of the target image
    // if all samples are rendered in the range [0:SAMPLE_RATE]
    fn build_buffer(lower_value: usize, upper_value: usize) -> gst::Buffer {
        let lower: SampleIndex = lower_value.into();
        let pts = Timestamp::new(lower.as_ts(SAMPLE_DURATION).as_u64() + 1);
        let samples_u8_len = (upper_value - lower_value) * CHANNELS * 2;

        let mut buffer = gst::Buffer::with_size(samples_u8_len).unwrap();
        {
            let buffer_mut = buffer.get_mut().unwrap();
            buffer_mut.set_pts(gst::ClockTime::from(pts));

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

    fn push_test_buffer(
        audio_buffer: &mut AudioBuffer,
        buffer: &gst::Buffer,
        is_new_segment: bool,
    ) {
        if is_new_segment {
            let mut segment = gst::FormattedSegment::new();
            segment.set_start(buffer.pts());
            segment.set_time(buffer.pts());
            audio_buffer.have_gst_segment(&segment);
        }

        audio_buffer.push_gst_buffer(buffer, SampleIndex::default()); // never drain buffer in this test
    }

    macro_rules! check_values(
        ($audio_buffer:expr, $idx:expr, $expected:expr) => (
            assert_eq!(
                $audio_buffer.get($idx),
                Some(&[SampleValue::from($expected), SampleValue::from(-$expected)][..])
            );
        );
    );

    macro_rules! check_last_values(
        ($audio_buffer:expr, $expected:expr) => (
            let mut last = $audio_buffer.upper;
            last.try_dec().expect("checking last values");
            check_values!($audio_buffer, last, $expected);
        );
    );

    #[test]
    fn multiple_gst_buffers() {
        //env_logger::init();
        gst::init().unwrap();

        let mut audio_buffer = AudioBuffer::new(Duration::from_secs(1));
        audio_buffer.init(
            &gst_audio::AudioInfo::builder(AUDIO_FORMAT_S16, SAMPLE_RATE, CHANNELS as u32)
                .build()
                .unwrap(),
        );

        info!("samples [100:200] init");
        push_test_buffer(&mut audio_buffer, &build_buffer(100, 200), true);
        assert_eq!(audio_buffer.lower, SampleIndex::new(100));
        assert_eq!(audio_buffer.upper, SampleIndex::new(200));
        check_values!(audio_buffer, audio_buffer.lower, 100);
        check_last_values!(audio_buffer, 199);

        info!("samples [50:100]: appending to the begining");
        push_test_buffer(&mut audio_buffer, &build_buffer(50, 100), true);
        assert_eq!(audio_buffer.lower, SampleIndex::new(50));
        assert_eq!(audio_buffer.upper, SampleIndex::new(200));
        check_values!(audio_buffer, audio_buffer.lower, 50);
        check_last_values!(audio_buffer, 199);

        info!("samples [0:75]: overlaping on the begining");
        push_test_buffer(&mut audio_buffer, &build_buffer(0, 75), true);
        assert_eq!(audio_buffer.lower, SampleIndex::new(0));
        assert_eq!(audio_buffer.upper, SampleIndex::new(200));
        check_values!(audio_buffer, audio_buffer.lower, 0);
        check_last_values!(audio_buffer, 199);

        info!("samples [200:300]: appending to the end - different segment");
        push_test_buffer(&mut audio_buffer, &build_buffer(200, 300), true);
        assert_eq!(audio_buffer.lower, SampleIndex::new(0));
        assert_eq!(audio_buffer.upper, SampleIndex::new(300));
        check_values!(audio_buffer, audio_buffer.lower, 0);
        check_last_values!(audio_buffer, 299);

        info!("samples [250:275]: contained in current - different segment");
        push_test_buffer(&mut audio_buffer, &build_buffer(250, 275), true);
        assert_eq!(audio_buffer.lower, SampleIndex::new(0));
        assert_eq!(audio_buffer.upper, SampleIndex::new(300));
        check_values!(audio_buffer, audio_buffer.lower, 0);
        check_last_values!(audio_buffer, 299);

        info!("samples [275:400]: overlaping on the end");
        push_test_buffer(&mut audio_buffer, &build_buffer(275, 400), false);
        assert_eq!(audio_buffer.lower, SampleIndex::new(0));
        assert_eq!(audio_buffer.upper, SampleIndex::new(400));
        check_values!(audio_buffer, audio_buffer.lower, 0);
        check_last_values!(audio_buffer, 399);

        info!("samples [400:450]: appending to the end");
        push_test_buffer(&mut audio_buffer, &build_buffer(400, 450), false);
        assert_eq!(audio_buffer.lower, SampleIndex::new(0));
        assert_eq!(audio_buffer.upper, SampleIndex::new(450));
        check_values!(audio_buffer, audio_buffer.lower, 0);
        check_last_values!(audio_buffer, 449);
    }

    fn check_iter(
        audio_buffer: &AudioBuffer,
        lower: usize,
        upper: usize,
        step: usize,
        expected_values: &[i16],
    ) {
        let lower = SampleIndex::new(lower);
        let upper = SampleIndex::new(upper);
        let step = SampleIndexRange::new(step);

        debug!("checking iter for [{}, {}], step {}...", lower, upper, step);
        let sample_iter = audio_buffer
            .try_iter(lower, upper, step)
            .expect("checking iter (test)");

        for (sample, expected_value) in sample_iter.zip(expected_values.iter()) {
            let expected_value = *expected_value;
            for (channel, value) in sample.iter().enumerate() {
                if channel == 0 {
                    assert_eq!(*value, SampleValue::from(expected_value));
                } else {
                    assert_eq!(*value, SampleValue::from(-expected_value));
                }
            }
        }

        debug!("... done");
    }

    #[test]
    fn test_iter() {
        //env_logger::init();
        gst::init().unwrap();

        let mut audio_buffer = AudioBuffer::new(Duration::from_secs(1));
        audio_buffer.init(
            &gst_audio::AudioInfo::builder(AUDIO_FORMAT_S16, SAMPLE_RATE, CHANNELS as u32)
                .build()
                .unwrap(),
        );

        info!("* samples [100:200] init");
        // 1. init
        push_test_buffer(&mut audio_buffer, &build_buffer(100, 200), true);

        // buffer ranges: front: [, ], back: [100, 200]
        // check bounds
        check_iter(&audio_buffer, 100, 110, 5, &[100, 105]);
        check_iter(&audio_buffer, 196, 200, 3, &[196, 199]);

        // 2. appending to the beginning
        push_test_buffer(&mut audio_buffer, &build_buffer(50, 100), true);

        // buffer ranges: front: [50, 100], back: [100, 200]
        // check beginning
        check_iter(&audio_buffer, 50, 60, 5, &[50, 55]);

        // check overlap between 1 & 2
        check_iter(&audio_buffer, 90, 110, 5, &[90, 95, 100, 105]);

        // 3. appending to the beginning
        push_test_buffer(&mut audio_buffer, &build_buffer(0, 75), true);

        // buffer ranges: front: [0, 100], back: [100, 200]

        // check overlap between 2 & 3
        check_iter(&audio_buffer, 40, 60, 5, &[40, 45, 50, 55]);

        // appending to the end
        // 4
        push_test_buffer(&mut audio_buffer, &build_buffer(200, 300), true);

        // buffer ranges: front: [0, 100], back: [100, 300]

        // check overlap between 1 & 4
        check_iter(&audio_buffer, 190, 210, 5, &[190, 195, 200, 205]);

        // 5 append in same segment
        push_test_buffer(&mut audio_buffer, &build_buffer(300, 400), false);

        // buffer ranges: front: [0, 100], back: [100, 400]
        // check end
        check_iter(&audio_buffer, 396, 400, 3, &[396, 399]);

        // check overlap between 4 & 5
        check_iter(&audio_buffer, 290, 310, 5, &[290, 295, 300, 305]);

        // check overlap between 4 & 5
        check_iter(&audio_buffer, 290, 310, 5, &[290, 295, 300, 305]);
    }
}
