extern crate gstreamer as gst;

#[cfg(feature = "profiling-audio-buffer")]
use chrono::Utc;

use byte_slice_cast::AsSliceOf;

use std::collections::vec_deque::VecDeque;

#[cfg(test)]
use byteorder::{ByteOrder, LittleEndian};

pub struct AudioBuffer {
    buffer_duration: u64,
    capacity: usize,
    pub sample_duration: u64,
    pub duration_for_1000_samples: f64,
    pub channels: usize,
    drain_size: usize,

    pub eos: bool,

    segement_lower: usize,
    last_buffer_pts: u64,
    last_buffer_upper: usize,
    pub lower: usize,
    pub upper: usize,
    pub samples: VecDeque<i16>,
}

impl AudioBuffer {
    pub fn new(
        buffer_duration: u64,
    ) -> Self
    {
        AudioBuffer {
            buffer_duration: buffer_duration,
            capacity: 0,
            sample_duration: 0,
            duration_for_1000_samples: 0f64,
            channels: 0,
            drain_size: 0,

            eos: false,

            segement_lower: 0,
            last_buffer_pts: 0,
            last_buffer_upper: 0,
            lower: 0,
            upper: 0,
            samples: VecDeque::new(),
        }
    }

    pub fn set_caps(&mut self, caps: &gst::Caps) {
        let structure = caps.get_structure(0)
            .expect("Couldn't get structure from audio caps");
        let rate = structure.get::<i32>("rate")
            .expect("Couldn't get rate from audio caps");

        // assert_eq!(format, S16);
        // assert_eq!(layout, Interleaved);
        self.channels = structure.get::<i32>("channels")
            .expect("Couldn't get channels from audio sample")
            as usize;

        self.sample_duration = 1_000_000_000 / (rate as u64);
        self.duration_for_1000_samples = 1_000_000_000_000f64 / (rate as f64);
        self.capacity =
            (self.buffer_duration / self.sample_duration) as usize
            * self.channels;
        self.samples = VecDeque::with_capacity(self.capacity);
        self.drain_size = rate as usize * self.channels; // 1s worth of samples
    }

    pub fn cleanup(&mut self) {
        self.eos = false;
        self.segement_lower = 0;
        self.last_buffer_pts = 0;
        self.last_buffer_upper = 0;
        self.lower = 0;
        self.upper = 0;
        self.channels = 0;
        self.sample_duration = 0;
        self.duration_for_1000_samples = 0f64;
        self.capacity = 0;
        self.samples.clear();
        self.drain_size = 0;
    }

    // Add samples from the GStreamer pipeline to the AudioBuffer
    // This buffer stores the complete set of samples in a time frame
    // in order to be able to represent the audio at any given precision.
    // Samples are stores as f64 suitable for on screen rendering.
    // Incoming samples are merged to the existing buffer when possible
    pub fn push_gst_sample(&mut self,
        sample: gst::Sample,
        lower_to_keep: usize,
    ) {
        #[cfg(feature = "profiling-audio-buffer")]
        let start = Utc::now();

        let buffer = sample.get_buffer()
            .expect("Couldn't get buffer from audio sample");

        let buffer_map = buffer.map_readable();
        let incoming_samples = buffer_map.as_ref().unwrap()
            .as_slice().as_slice_of::<i16>()
            .expect("Couldn't get audio samples as i16");

        self.eos = false;

        let segment_lower = (
            sample.get_segment().unwrap().get_start() / self.sample_duration
        ) as usize;
        let buffer_sample_len = incoming_samples.len() / self.channels;
        let buffer_pts = buffer.get_pts();

        // TODO: it seems that caps might change during playback.
        // Segment gives access to caps, so it might be a way
        // to monitor a caps modification

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
        let (
            lower_changed,
            incoming_lower,
            lower_to_add_rel,
            upper_to_add_rel
        ) =
            if !self.samples.is_empty()
            {   // not initializing
                let incoming_lower =
                    if segment_lower == self.segement_lower {
                        // receiving next buffer in the same segment
                        if buffer_pts > self.last_buffer_pts {
                            // ... and getting a more recent buffer than previous
                            // => assuming incoming buffer comes just after previous
                            self.last_buffer_upper
                        } else {
                            // ... but incoming buffer is ealier in the stream
                            // => probably a seek back to the begining
                            // of current segment
                            // (e.g. seeking at the begining of current chapter)
                            segment_lower
                        }
                    } else {
                        // different segment (done seeking)
                        self.segement_lower = segment_lower;
                        self.last_buffer_upper = segment_lower;
                        segment_lower
                    };
                let incoming_upper = incoming_lower + buffer_sample_len;

                if incoming_lower == self.upper {
                    // 1. append incoming buffer to the end of internal storage
                    #[cfg(test)]
                    println!("AudioBuffer case 1. appending to the end (full)");
                    // self.lower unchanged
                    self.upper = incoming_upper;
                    self.last_buffer_upper = incoming_upper;

                    (
                        false,            // lower_changed
                        incoming_lower,   // incoming_lower
                        0,                // lower_to_add_rel
                        buffer_sample_len // upper_to_add_rel
                    )
                } else if incoming_lower >= self.lower
                && incoming_upper <= self.upper {
                    // 2. incoming buffer included in current container
                    #[cfg(test)]
                    println!("AudioBuffer case 2. contained in current container");
                    self.last_buffer_upper = incoming_upper;
                    (
                        false,           // lower_changed
                        incoming_lower,  // incoming_lower
                        0,               // lower_to_add_rel
                        0                // upper_to_add_rel
                    )
                } else if incoming_lower > self.lower
                && incoming_lower < self.upper
                {   // 3. can append [self.upper, upper] to the end
                    #[cfg(test)]
                    println!("AudioBuffer case 3. append to the end (partial)");
                    // self.lower unchanged
                    let previous_upper = self.upper;
                    self.upper = incoming_upper;
                    // self.first_pts unchanged
                    self.last_buffer_upper = incoming_upper;
                    (
                        false,                           // lower_changed
                        incoming_lower,                  // incoming_lower
                        previous_upper - incoming_lower, // lower_to_add_rel
                        buffer_sample_len                // upper_to_add_rel
                    )
                }
                else if incoming_upper < self.upper
                && incoming_upper >= self.lower
                {   // 4. can insert [lower, self.lower] at the begining
                    #[cfg(test)]
                    println!("AudioBuffer case 4. insert at the begining");
                    let upper_to_add = self.lower;
                    self.lower = incoming_lower;
                    // self.upper unchanged
                    self.last_buffer_upper = incoming_upper;
                    (
                        true,                         // lower_changed
                        incoming_lower,               // incoming_lower
                        0,                            // lower_to_add_rel
                        upper_to_add - incoming_lower // upper_to_add_rel
                    )
                } else {
                    // 5. can't merge with previous buffer
                    #[cfg(test)]
                    println!("AudioBuffer case 5. can't merge self [{}, {}], incoming [{}, {}]",
                        self.lower, self.upper, incoming_lower, incoming_upper
                    );
                    self.samples.clear();
                    self.lower = incoming_lower;
                    self.upper = incoming_upper;
                    self.last_buffer_upper = incoming_upper;
                    (
                        true,               // lower_changed
                        incoming_lower,     // incoming_lower
                        0,                  // lower_to_add_rel
                        buffer_sample_len   // upper_to_add_rel
                    )
                }
            } else {
                // 6. initializing
                #[cfg(test)]
                println!("AudioBuffer init");
                self.segement_lower = segment_lower;
                self.lower = segment_lower;
                self.upper = segment_lower + buffer_sample_len;
                self.last_buffer_upper = self.upper;
                (
                    true,                   // lower_changed
                    segment_lower,          // incoming_lower
                    0,                      // lower_to_add_rel
                    buffer_sample_len       // upper_to_add_rel
                )
            };

        self.last_buffer_pts = buffer_pts;

        #[cfg(feature = "profiling-audio-buffer")]
        let before_drain = Utc::now();

        // drain internal buffer if necessary and possible
        if !lower_changed
        && self.samples.len()
            + (upper_to_add_rel - lower_to_add_rel) * self.channels
            > self.capacity
        {   // don't drain if samples are to be added at the begining...
            // drain only if we have enough samples in history
            // TODO: it could be worth testing truncate instead
            // (this would require reversing the buffer alimentation
            // and iteration)

            // Don't drain samples if they might be used by the extractor
            // (limit known as argument lower_to_keep)
            if lower_to_keep.min(incoming_lower)
                > self.lower + self.drain_size / self.channels
            {
                //println!("draining... len before: {}", self.samples.len());
                self.samples.drain(..self.drain_size);
                self.lower += self.drain_size / self.channels;
            }
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
                    self.samples.place_back() <- *sample_byte;
                };
            } else {
                let rev_sample_iter = sample_slice.iter().rev();
                for sample_byte in rev_sample_iter {
                    self.samples.place_front() <- *sample_byte;
                }
            }
        }

        #[cfg(feature = "profiling-audio-buffer")]
        let end = Utc::now();

        #[cfg(feature = "profiling-audio-buffer")]
        println!("audio-buffer,{},{},{},{}",
            start.time().format("%H:%M:%S%.6f"),
            before_drain.time().format("%H:%M:%S%.6f"),
            before_storage.time().format("%H:%M:%S%.6f"),
            end.time().format("%H:%M:%S%.6f"),
        );
    }

    pub fn handle_eos(&mut self) {
        if !self.samples.is_empty() {
            self.eos = true;
        }
    }

    pub fn iter(&self,
        lower: usize,
        upper: usize,
        sample_step: usize,
        channel: usize,
    ) -> Option<Iter> {
        Iter::new(self, lower, upper, sample_step, channel)
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
    pub fn push_samples(&mut self,
        samples: &[i16],
        lower: usize,
        segment_lower: usize,
        caps: &gst::Caps,
    ) {
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
        };

        let mut buffer = gst::Buffer::from_vec(samples_u8).unwrap();
        buffer.get_mut().unwrap().set_pts(
            self.sample_duration * (lower as u64) + 1
        );

        let mut segment = gst::Segment::new();
        segment.set_start(
            self.sample_duration * (segment_lower as u64) + 1
        );

        let self_lower = self.lower;
        self.push_gst_sample(
            gst::Sample::new(Some(buffer), Some(caps.clone()), Some(&segment), None),
            self_lower // never drain buffer in this test
        );
    }
}

pub struct Iter<'a> {
    slice0: &'a [i16],
    slice0_len: usize,
    slice1: &'a [i16],
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
        channel: usize,
    ) -> Option<Iter<'a>> {
        if upper > lower
        && lower >= buffer.lower
        && upper <= buffer.upper
        && channel < buffer.channels
        {
            let slices = buffer.samples.as_slices();
            let len0 = slices.0.len();
            Some(Iter {
                slice0: slices.0,
                slice0_len: len0,
                slice1: slices.1,
                idx: (lower - buffer.lower) * buffer.channels + channel,
                upper: (upper - buffer.lower) * buffer.channels + channel,
                step: sample_step * buffer.channels,
            })
        } else {
            // out of bound TODO: return an error
            #[cfg(test)]
            println!("AudioBuffer::Iter::new [{}, {}] out of bounds [{}, {}]",
                lower, upper, buffer.lower, buffer.upper
            );
            None
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.upper {
            return None;
        }

        let item =
            if self.idx < self.slice0_len {
                self.slice0[self.idx]
            } else {
                self.slice1[self.idx - self.slice0_len]
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
    extern crate gstreamer_audio as gst_audio;

    use media::AudioBuffer;

    const SAMPLE_RATE: i32 = 300;

    #[test]
    fn multiple_gst_samples() {
        gst::init().unwrap();

        let mut audio_buffer = AudioBuffer::new(1_000_000_000); // 1s
        let caps = gst::Caps::new_simple(
            "audio/x-raw",
            &[
                ("format", &gst_audio::AUDIO_FORMAT_S16.to_string()),
                ("layout", &"interleaved"),
                ("channels", &2),
                ("rate", &SAMPLE_RATE),
            ],
        );
        audio_buffer.set_caps(&caps);

        // Build a buffer 2 channels in the specified range
        // which would be rendered as a diagonal on a Waveform image
        // from left top corner to right bottom of the target image
        // if all samples are rendered in the range [0:SAMPLE_RATE]
        fn build_buffer(lower: usize, upper: usize) -> Vec<i16> {
            let mut buffer: Vec<i16> = Vec::new();
            for index in lower..upper {
                buffer.push(index as i16);
                buffer.push(-(index as i16)); // second channel <= opposite value
            }
            buffer
        }

        println!("\n* samples [100:200] init");
        audio_buffer.push_samples(&build_buffer(100, 200), 100, 100, &caps);
        assert_eq!(audio_buffer.lower, 100);
        assert_eq!(audio_buffer.upper, 200);
        assert_eq!(
            audio_buffer.get(audio_buffer.lower),
            Some(&[100, -100][..])
        );
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[199, -199][..])
        );

        println!("* samples [50:100]: appending to the begining");
        audio_buffer.push_samples(&build_buffer(50, 100), 50, 50, &caps);
        assert_eq!(audio_buffer.lower, 50);
        assert_eq!(audio_buffer.upper, 200);
        assert_eq!(
            audio_buffer.get(audio_buffer.lower),
            Some(&[50, -50][..])
        );
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[199, -199][..])
        );

        println!("* samples [0:75]: overlaping on the begining");
        audio_buffer.push_samples(&build_buffer(0, 75), 0, 0, &caps);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 200);
        assert_eq!(
            audio_buffer.get(audio_buffer.lower),
            Some(&[0, 0][..])
        );
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[199, -199][..])
        );

        println!("* samples [200:300]: appending to the end - different segment");
        audio_buffer.push_samples(&build_buffer(200, 300), 200, 200, &caps);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 300);
        assert_eq!(
            audio_buffer.get(audio_buffer.lower),
            Some(&[0, 0][..])
        );
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[299, -299][..])
        );

        println!("* samples [250:275]: contained in current - different segment");
        audio_buffer.push_samples(&build_buffer(250, 275), 250, 250, &caps);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 300);
        assert_eq!(
            audio_buffer.get(audio_buffer.lower),
            Some(&[0, 0][..])
        );
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[299, -299][..])
        );

        println!("* samples [275:400]: overlaping on the end");
        audio_buffer.push_samples(&build_buffer(275, 400), 275, 250, &caps);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 400);
        assert_eq!(
            audio_buffer.get(audio_buffer.lower),
            Some(&[0, 0][..])
        );
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[399, -399][..])
        );

        println!("* samples [400:450]: appending to the end");
        audio_buffer.push_samples(&build_buffer(400, 450), 400, 250, &caps);
        assert_eq!(audio_buffer.lower, 0);
        assert_eq!(audio_buffer.upper, 450);
        assert_eq!(
            audio_buffer.get(audio_buffer.lower),
            Some(&[0, 0][..])
        );
        assert_eq!(
            audio_buffer.get(audio_buffer.upper - 1),
            Some(&[449, -449][..])
        );
    }
}
