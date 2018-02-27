extern crate gstreamer as gst;

#[cfg(feature = "profiling-audio-buffer")]
use chrono::Utc;

use byte_slice_cast::AsSliceOf;

use std::collections::vec_deque::VecDeque;

#[cfg(test)]
extern crate gstreamer_audio as gst_audio;

#[cfg(test)]
use gstreamer::ClockTime;

#[cfg(test)]
use byteorder::{ByteOrder, LittleEndian};

pub struct AudioBuffer {
    buffer_duration: u64,
    capacity: usize,
    rate: u64,
    pub sample_duration: u64,
    pub channels: usize,
    drain_size: usize,

    pub eos: bool,

    is_new_segment: bool,
    segment_start: Option<u64>,
    pub segment_lower: usize,
    last_buffer_upper: usize,
    pub lower: usize,
    pub upper: usize,
    pub samples: VecDeque<i16>,
}

impl AudioBuffer {
    pub fn new(buffer_duration: u64) -> Self {
        AudioBuffer {
            buffer_duration: buffer_duration,
            capacity: 0,
            rate: 0,
            sample_duration: 0,
            channels: 0,
            drain_size: 0,

            eos: false,

            is_new_segment: true,
            segment_start: None,
            segment_lower: 0,
            last_buffer_upper: 0,
            lower: 0,
            upper: 0,
            samples: VecDeque::new(),
        }
    }

    pub fn init(&mut self, rate: u64, channels: usize) {
        // assert_eq!(format, S16);
        // assert_eq!(layout, Interleaved);

        // changing caps
        self.cleanup();

        self.rate = rate;
        self.sample_duration = 1_000_000_000 / rate;
        self.channels = channels;
        self.capacity = (self.buffer_duration / self.sample_duration) as usize * self.channels;
        self.samples = VecDeque::with_capacity(self.capacity);
        self.drain_size = rate as usize * self.channels; // 1s worth of samples
    }

    // Clean everytihng so that the AudioBuffer
    // can be reused for a different media
    pub fn cleanup(&mut self) {
        #[cfg(any(test, feature = "trace-audio-buffer"))]
        println!("\nAudioBuffer cleaning up");

        self.reset();
        self.segment_start = None;
    }

    // Reset the AudioBuffer keeping continuity
    // This is required in case of a caps change or stream change
    // as samples may come in the same segment despite the change.
    // If the media is paused and then set back to playback, preroll
    // will be performed in the same segment as before the change.
    // So we need to keep track of the segment start in order not
    // to reset current sequence (self.last_buffer_upper).
    pub fn reset(&mut self) {
        #[cfg(any(test, feature = "trace-audio-buffer"))]
        println!("\nAudioBuffer resetting");

        self.capacity = 0;
        self.rate = 0;
        self.sample_duration = 0;
        self.channels = 0;
        self.drain_size = 0;
        self.eos = false;
        self.is_new_segment = true;
        // don't cleanup self.segment_start in order to maintain continuity
        self.last_buffer_upper = 0;
        self.segment_lower = 0;
        self.lower = 0;
        self.upper = 0;
        self.samples.clear();
    }

    // Add samples from the GStreamer pipeline to the AudioBuffer
    // This buffer stores the complete set of samples in a time frame
    // in order to be able to represent the audio at any given precision.
    // Incoming samples are merged to the existing buffer when possible
    // Returns: number of samples received
    pub fn push_gst_buffer(&mut self, buffer: &gst::Buffer, lower_to_keep: usize) -> usize {
        let pts = buffer.get_pts().unwrap();

        if buffer.get_flags() & gst::BufferFlags::DISCONT == gst::BufferFlags::DISCONT {
            // reached a discontinuity
            if let Some(current_segment_start) = self.segment_start {
                if current_segment_start != pts {
                    self.segment_lower = (pts / self.sample_duration) as usize;
                    self.last_buffer_upper = self.segment_lower;
                }
                // else: same segment => might be an async_done after a pause
                //       or a seek back to the segment's start
            }

            self.segment_start = Some(pts);
        }

        let buffer_map = buffer.map_readable();
        let incoming_samples = buffer_map
            .as_ref()
            .unwrap()
            .as_slice()
            .as_slice_of::<i16>()
            .expect("AudioBuffer::preroll_gst_sample couldn't get audio samples as i16");
        let buffer_sample_len = incoming_samples.len() / self.channels;

        // Unfortunately, we can't rely on buffer_pts to figure out
        // the exact position in the segment. Some streams use a pts
        // value which is a rounded value to e.g ms and correct
        // the shift every n samples.
        // After an accurate seek, the buffer pts seems reliable,
        // however after a inaccurate seek, we get a rounded value.
        // The strategy here is to consider that each incoming buffer
        // in the same segment comes after last buffer (we might need
        // to check buffer drops for this) and in case of a new segment
        // we'll rely on the inaccurate pts value...

        if self.is_new_segment {
            self.is_new_segment = false;
        }

        let incoming_lower = self.last_buffer_upper;
        let incoming_upper = incoming_lower + buffer_sample_len;

        // Identify conditions for this incoming buffer:
        // 1. Incoming buffer fits at the end of current container.
        // 2. Incoming buffer is already contained within stored samples.
        //    Nothing to do.
        // 3. Incoming buffer overlaps with stored samples at the end.
        // 4. Incoming buffer overlaps with stored samples at the begining.
        //    Note: this changes the lower sample and requires to extend
        //    the internal container from the begining.
        // 5. Incoming buffer doesn't overlap with current buffer. In order
        //    not to let gaps between samples, the internal container is
        //    cleared lower.
        // 6. The internal container is empty, import incoming buffer
        //    completely.
        let (lower_changed, incoming_lower, lower_to_add_rel, upper_to_add_rel) =
            if !self.samples.is_empty() {
                // not initializing
                if incoming_lower == self.upper {
                    // 1. append incoming buffer to the end of internal storage
                    #[cfg(test)]
                    println!("AudioBuffer case 1. appending to the end (full)");
                    // self.lower unchanged
                    self.upper = incoming_upper;
                    self.eos = false;
                    self.last_buffer_upper = incoming_upper;

                    (
                        false,             // lower_changed
                        incoming_lower,    // incoming_lower
                        0,                 // lower_to_add_rel
                        buffer_sample_len, // upper_to_add_rel
                    )
                } else if incoming_lower >= self.lower && incoming_upper <= self.upper {
                    // 2. incoming buffer included in current container
                    #[cfg(any(test, feature = "trace-audio-buffer"))]
                    println!(
                        concat!(
                            "AudioBuffer case 2. contained in current container ",
                            "self [{}, {}], incoming [{}, {}]",
                        ),
                        self.lower,
                        self.upper,
                        incoming_lower,
                        incoming_upper
                    );
                    self.last_buffer_upper = incoming_upper;
                    (
                        false,          // lower_changed
                        incoming_lower, // incoming_lower
                        0,              // lower_to_add_rel
                        0,              // upper_to_add_rel
                    )
                } else if incoming_lower > self.lower && incoming_lower < self.upper {
                    // 3. can append [self.upper, upper] to the end
                    #[cfg(any(test, feature = "trace-audio-buffer"))]
                    println!(
                        concat!(
                            "AudioBuffer case 3. append to the end (partial) ",
                            "[{}, {}], incoming [{}, {}]",
                        ),
                        self.upper,
                        incoming_upper,
                        incoming_lower,
                        incoming_upper
                    );
                    // self.lower unchanged
                    let previous_upper = self.upper;
                    self.upper = incoming_upper;
                    self.eos = false;
                    // self.first_pts unchanged
                    self.last_buffer_upper = incoming_upper;
                    (
                        false,                           // lower_changed
                        incoming_lower,                  // incoming_lower
                        previous_upper - incoming_lower, // lower_to_add_rel
                        buffer_sample_len,               // upper_to_add_rel
                    )
                } else if incoming_upper < self.upper && incoming_upper >= self.lower {
                    // 4. can insert [lower, self.lower] at the begining
                    #[cfg(any(test, feature = "trace-audio-buffer"))]
                    println!(
                        "AudioBuffer case 4. insert at the begining [{}, {}], incoming [{}, {}]",
                        incoming_lower, self.lower, incoming_lower, incoming_upper
                    );
                    let upper_to_add = self.lower;
                    self.lower = incoming_lower;
                    // self.upper unchanged
                    self.last_buffer_upper = incoming_upper;
                    (
                        true,                          // lower_changed
                        incoming_lower,                // incoming_lower
                        0,                             // lower_to_add_rel
                        upper_to_add - incoming_lower, // upper_to_add_rel
                    )
                } else {
                    // 5. can't merge with previous buffer
                    #[cfg(any(test, feature = "trace-audio-buffer"))]
                    println!(
                        "AudioBuffer case 5. can't merge self [{}, {}], incoming [{}, {}]",
                        self.lower, self.upper, incoming_lower, incoming_upper
                    );
                    self.samples.clear();
                    self.lower = incoming_lower;
                    self.upper = incoming_upper;
                    self.eos = false;
                    self.last_buffer_upper = incoming_upper;
                    (
                        true,              // lower_changed
                        incoming_lower,    // incoming_lower
                        0,                 // lower_to_add_rel
                        buffer_sample_len, // upper_to_add_rel
                    )
                }
            } else {
                // 6. initializing
                #[cfg(any(test, feature = "trace-audio-buffer"))]
                println!("AudioBuffer init [{}, {}]", incoming_lower, incoming_upper);
                self.lower = incoming_lower;
                self.upper = incoming_upper;
                self.eos = false;
                self.last_buffer_upper = self.upper;
                (
                    true,              // lower_changed
                    incoming_lower,    // incoming_lower
                    0,                 // lower_to_add_rel
                    buffer_sample_len, // upper_to_add_rel
                )
            };

        #[cfg(feature = "profiling-audio-buffer")]
        let before_drain = Utc::now();

        // Don't drain if samples are to be added at the begining...
        // drain only if we have enough samples in history
        // TODO: it could be worth testing truncate instead
        // (this would require reversing the buffer alimentation
        // and iteration).
        // Don't drain samples if they might be used by the extractor
        // (limit known as argument lower_to_keep).
        if !lower_changed
            && self.samples.len() + (upper_to_add_rel - lower_to_add_rel) * self.channels
                > self.capacity
            && lower_to_keep.min(incoming_lower) > self.lower + self.drain_size / self.channels
        {
            //println!("draining... len before: {}", self.samples.len());
            self.samples.drain(..self.drain_size);
            self.lower += self.drain_size / self.channels;
        }

        #[cfg(feature = "profiling-audio-buffer")]
        let before_storage = Utc::now();

        if upper_to_add_rel > 0 {
            let lower_idx = lower_to_add_rel * self.channels;
            let upper_idx = upper_to_add_rel * self.channels;
            let sample_slice = &incoming_samples[lower_idx..upper_idx];

            if !lower_changed || self.samples.is_empty() {
                // samples can be pushed back to the containr
                for sample_byte in sample_slice.iter() {
                    self.samples.push_back(*sample_byte);
                }
            } else {
                let rev_sample_iter = sample_slice.iter().rev();
                for sample_byte in rev_sample_iter {
                    self.samples.push_front(*sample_byte);
                }
            }
        }

        #[cfg(feature = "profiling-audio-buffer")]
        let end = Utc::now();

        #[cfg(feature = "profiling-audio-buffer")]
        println!(
            "audio-buffer,{},{},{},{}",
            start.time().format("%H:%M:%S%.6f"),
            before_drain.time().format("%H:%M:%S%.6f"),
            before_storage.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
        );

        buffer_sample_len // nb of samples received
    }

    pub fn handle_eos(&mut self) {
        // EOS can be received when seeking within a range
        // in our case, this occurs in paused mode at the end of a range playback.
        // In this situation, the last samples received should already be contained
        // in the AudioBuffer.
        if !self.samples.is_empty() {
            self.eos = true;
        }
        self.segment_start = None;
    }

    pub fn iter(&self, lower: usize, upper: usize, sample_step: usize) -> Option<Iter> {
        Iter::new(self, lower, upper, sample_step)
    }

    pub fn get(&self, sample: usize) -> Option<&[i16]> {
        if sample >= self.lower && sample < self.upper {
            let slices = self.samples.as_slices();
            let slice0_len = slices.0.len();
            let mut idx = (sample - self.lower) * self.channels;
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

    #[cfg(test)]
    pub fn push_samples(&mut self, samples: &[i16], lower: usize, is_new_segment: bool) {
        let mut samples_u8 = Vec::with_capacity(samples.len() * 2 * self.channels);
        let mut buf_u8 = [0; 2];

        let mut iter = samples.iter();
        let mut value = iter.next();
        while value.is_some() {
            for _channel in 0..self.channels {
                LittleEndian::write_i16(&mut buf_u8, *value.unwrap());

                samples_u8.push(buf_u8[0]);
                samples_u8.push(buf_u8[1]);

                value = iter.next()
            }
        }

        let mut buffer = gst::Buffer::with_size(samples_u8.len()).unwrap();
        {
            let buffer_mut = buffer.get_mut().unwrap();
            buffer_mut.copy_from_slice(0, &samples_u8).unwrap();
            buffer_mut.set_pts(ClockTime::from(self.sample_duration * (lower as u64) + 1));
        }

        let mut segment = gst::Segment::new();
        segment.set_format(gst::Format::Time);
        segment.set_start(ClockTime::from_nseconds(
            self.sample_duration * (lower as u64) + 1,
        ));

        let caps = gst::Caps::new_simple(
            "audio/x-raw",
            &[
                ("format", &gst_audio::AUDIO_FORMAT_S16.to_string()),
                ("layout", &"interleaved"),
                ("channels", &(self.channels as i32)),
                ("rate", &(self.rate as i32)),
            ],
        );

        let sample = gst::Sample::new(Some(&buffer), Some(&caps), Some(&segment), None);
        if is_new_segment {
            self.preroll_gst_sample(&sample);
        }
        let self_lower = self.lower;
        self.push_gst_sample(&sample, self_lower); // never drain buffer in this test
    }
}

pub struct Iter<'a> {
    slice0: &'a [i16],
    slice0_len: usize,
    slice1: &'a [i16],
    channels: usize,
    idx: usize,
    upper: usize,
    step: usize,
}

impl<'a> Iter<'a> {
    fn new(
        buffer: &'a AudioBuffer,
        lower: usize,
        upper: usize,
        sample_step: usize,
    ) -> Option<Iter<'a>> {
        if upper > lower && lower >= buffer.lower && upper <= buffer.upper {
            let slices = buffer.samples.as_slices();
            let len0 = slices.0.len();
            Some(Iter {
                slice0: slices.0,
                slice0_len: len0,
                slice1: slices.1,
                channels: buffer.channels,
                idx: (lower - buffer.lower) * buffer.channels,
                upper: (upper - buffer.lower) * buffer.channels,
                step: sample_step * buffer.channels,
            })
        } else {
            // out of bound TODO: return an error
            #[cfg(test)]
            println!(
                "AudioBuffer::Iter::new [{}, {}] out of bounds [{}, {}]",
                lower, upper, buffer.lower, buffer.upper
            );
            None
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a [i16];

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.upper {
            return None;
        }

        let item = if self.idx < self.slice0_len {
            &self.slice0[self.idx..self.idx + self.channels]
        } else {
            let idx = self.idx - self.slice0_len;
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
    extern crate gstreamer as gst;

    use media::AudioBuffer;

    const SAMPLE_RATE: u64 = 300;

    // Build a buffer with 2 channels in the specified range
    // which would be rendered as a diagonal on a Waveform image
    // from left top corner to right bottom of the target image
    // if all samples are rendered in the range [0:SAMPLE_RATE]
    fn build_buffer(lower_value: usize, upper_value: usize) -> Vec<i16> {
        let mut buffer: Vec<i16> = Vec::new();
        for index in lower_value..upper_value {
            buffer.push(index as i16);
            buffer.push(-(index as i16)); // second channel <= opposite value
        }
        buffer
    }

    #[test]
    fn multiple_gst_samples() {
        gst::init().unwrap();

        let mut audio_buffer = AudioBuffer::new(1_000_000_000); // 1s
        audio_buffer.init(SAMPLE_RATE, 2); //2 channels

        println!("\n* samples [100:200] init");
        audio_buffer.push_samples(&build_buffer(100, 200), 100, true);
        assert_eq!(audio_buffer.lower, 100);
        assert_eq!(audio_buffer.upper, 200);
        assert_eq!(audio_buffer.get(audio_buffer.lower), Some(&[100, -100][..]));
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[199, -199][..])
        );

        println!("* samples [50:100]: appending to the begining");
        audio_buffer.push_samples(&build_buffer(50, 100), 50, true);
        assert_eq!(audio_buffer.lower, 50);
        assert_eq!(audio_buffer.upper, 200);
        assert_eq!(audio_buffer.get(audio_buffer.lower), Some(&[50, -50][..]));
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[199, -199][..])
        );

        println!("* samples [0:75]: overlaping on the begining");
        audio_buffer.push_samples(&build_buffer(0, 75), 0, true);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 200);
        assert_eq!(audio_buffer.get(audio_buffer.lower), Some(&[0, 0][..]));
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[199, -199][..])
        );

        println!("* samples [200:300]: appending to the end - different segment");
        audio_buffer.push_samples(&build_buffer(200, 300), 200, true);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 300);
        assert_eq!(audio_buffer.get(audio_buffer.lower), Some(&[0, 0][..]));
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[299, -299][..])
        );

        println!("* samples [250:275]: contained in current - different segment");
        audio_buffer.push_samples(&build_buffer(250, 275), 250, true);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 300);
        assert_eq!(audio_buffer.get(audio_buffer.lower), Some(&[0, 0][..]));
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[299, -299][..])
        );

        println!("* samples [275:400]: overlaping on the end");
        audio_buffer.push_samples(&build_buffer(275, 400), 275, false);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 400);
        assert_eq!(audio_buffer.get(audio_buffer.lower), Some(&[0, 0][..]));
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[399, -399][..])
        );

        println!("* samples [400:450]: appending to the end");
        audio_buffer.push_samples(&build_buffer(400, 450), 400, false);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 450);
        assert_eq!(audio_buffer.get(audio_buffer.lower), Some(&[0, 0][..]));
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[449, -449][..])
        );
    }

    fn check_iter(
        audio_buffer: &AudioBuffer,
        lower: usize,
        upper: usize,
        step: usize,
        expected_values: &[i16],
    ) {
        println!(
            "\tchecking iter for [{}, {}], step {}...",
            lower, upper, step
        );
        let mut iter = audio_buffer.iter(lower, upper, step).unwrap();

        for expected_value in expected_values {
            let iter_next = iter.next();
            let channel_values = iter_next.unwrap();
            for (channel_id, channel_value) in channel_values.iter().enumerate() {
                if channel_id == 0 {
                    assert_eq!(expected_value, channel_value);
                } else {
                    assert_eq!(*expected_value, -1 * channel_value);
                }
            }
        }
        println!("\t... done");
    }

    #[test]
    fn test_iter() {
        gst::init().unwrap();

        let mut audio_buffer = AudioBuffer::new(1_000_000_000); // 1s
        audio_buffer.init(SAMPLE_RATE, 2); //2 channels

        println!("\n* samples [100:200] init");
        // 1. init
        audio_buffer.push_samples(&build_buffer(100, 200), 100, true);

        // buffer ranges: front: [, ], back: [100, 200]
        // check bounds
        check_iter(&audio_buffer, 100, 110, 5, &vec![100, 105]);
        check_iter(&audio_buffer, 196, 200, 3, &vec![196, 199]);

        // 2. appending to the beginning
        audio_buffer.push_samples(&build_buffer(50, 100), 50, true);

        // buffer ranges: front: [50, 100], back: [100, 200]
        // check beginning
        check_iter(&audio_buffer, 50, 60, 5, &vec![50, 55]);

        // check overlap between 1 & 2
        check_iter(&audio_buffer, 90, 110, 5, &vec![90, 95, 100, 105]);

        // 3. appending to the beginning
        audio_buffer.push_samples(&build_buffer(0, 75), 0, true);

        // buffer ranges: front: [0, 100], back: [100, 200]

        // check overlap between 2 & 3
        check_iter(&audio_buffer, 40, 60, 5, &vec![40, 45, 50, 55]);

        // appending to the end
        // 4
        audio_buffer.push_samples(&build_buffer(200, 300), 200, true);

        // buffer ranges: front: [0, 100], back: [100, 300]

        // check overlap between 1 & 4
        check_iter(&audio_buffer, 190, 210, 5, &vec![190, 195, 200, 205]);

        // 5 append in same segment
        audio_buffer.push_samples(&build_buffer(300, 400), 300, false);

        // buffer ranges: front: [0, 100], back: [100, 400]
        // check end
        check_iter(&audio_buffer, 396, 400, 3, &vec![396, 399]);

        // check overlap between 4 & 5
        check_iter(&audio_buffer, 290, 310, 5, &vec![290, 295, 300, 305]);

        // check overlap between 4 & 5
        check_iter(&audio_buffer, 290, 310, 5, &vec![290, 295, 300, 305]);
    }
}
