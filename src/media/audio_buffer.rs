extern crate gstreamer as gst;

#[cfg(feature = "profiling-audio-buffer")]
use chrono::Utc;

use byte_slice_cast::AsSliceOf;

use std::i16;

use std::collections::vec_deque::VecDeque;

#[cfg(test)]
use byteorder::{ByteOrder, LittleEndian};

pub const SAMPLES_NORM: f64 = 450f64;
const SAMPLES_OFFSET: f64 = SAMPLES_NORM / 2f64;

pub struct AudioBuffer {
    buffer_duration: u64,
    capacity: usize,
    pub sample_duration: f64,
    channels: usize,
    drain_size: usize,

    pub eos: bool,

    segement_first_sample: usize,
    last_buffer_pts: u64,
    last_buffer_last_sample: usize,
    pub first_sample: usize,
    pub last_sample: usize,
    pub samples: VecDeque<f64>,
}

impl AudioBuffer {
    pub fn new(
        buffer_duration: u64,
    ) -> Self
    {
        AudioBuffer {
            buffer_duration: buffer_duration,
            capacity: 0,
            sample_duration: 0f64,
            channels: 0,
            drain_size: 0,

            eos: false,

            segement_first_sample: 0,
            last_buffer_pts: 0,
            last_buffer_last_sample: 0,
            first_sample: 0,
            last_sample: 0,
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

        self.sample_duration = 1_000_000_000f64 / (rate as f64);
        self.capacity = (self.buffer_duration as f64 / self.sample_duration) as usize;
        self.samples = VecDeque::with_capacity(self.capacity);
        self.drain_size = (1_000_000_000f64 / self.sample_duration) as usize; // 1s worth of samples
    }

    pub fn cleanup(&mut self) {
        self.eos = false;
        self.segement_first_sample = 0;
        self.last_buffer_pts = 0;
        self.last_buffer_last_sample = 0;
        self.first_sample = 0;
        self.last_sample = 0;
        self.channels = 0;
        self.sample_duration = 0f64;
        self.capacity = 0;
        self.samples.clear();
        self.drain_size = 0;
    }

    // Add samples from the GStreamer pipeline to the AudioBuffer
    // This buffer stores the complete set of samples in a time frame
    // in order to be able to represent the audio at any given precision.
    // Samples are stores as f64 suitable for on screen rendering.
    // This implementation also performs a downmix.
    // TODO: make the representation dependent on the actual context
    //       move this as a trait and make the sample normalization
    //       part of a concrete object
    // Incoming samples are merged to the existing buffer when possible
    pub fn push_gst_sample(&mut self,
        sample: gst::Sample,
        first_sample_to_keep: usize,
    ) {
        #[cfg(feature = "profiling-audio-buffer")]
        let start = Utc::now();

        let buffer = sample.get_buffer()
            .expect("Couldn't get buffer from audio sample");

        let map = buffer.map_readable().unwrap();
        let incoming_samples = map.as_slice().as_slice_of::<i16>()
            .expect("Couldn't get audio samples as i16");

        #[cfg(feature = "profiling-audio-buffer")]
        let before_drain = Utc::now();

        self.eos = false;

        let segment_first_sample = (
            sample.get_segment().unwrap().get_start() as f64 / self.sample_duration
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
        //    Note: this changes the first sample and requires to extend
        //    the internal container from the begining.
        // 5. Incoming buffer doesn't overlap with current buffer. In order
        //    not to let gaps between samples, the internal container is
        //    cleared first.
        // 6. The internal container is empty, import incoming buffer
        //    completely.
        let (
            first_sample_changed,
            first_incoming_sample,
            first_sample_to_add_rel,
            last_sample_to_add_rel
        ) =
            if !self.samples.is_empty()
            {   // not initializing
                let first_incoming_sample =
                    if segment_first_sample == self.segement_first_sample {
                        // receiving next buffer in the same segment
                        if buffer_pts > self.last_buffer_pts {
                            // ... and getting a more recent buffer than previous
                            // => assuming incoming buffer comes just after previous
                            self.last_buffer_last_sample
                        } else {
                            // ... but incoming buffer is ealier in the stream
                            // => probably a seek back to the begining
                            // of current segment
                            // (e.g. seeking at the begining of current chapter)
                            segment_first_sample
                        }
                    } else {
                        // different segment (done seeking)
                        self.segement_first_sample = segment_first_sample;
                        self.last_buffer_last_sample = segment_first_sample;
                        println!("seg: {}, last {}, buffer last {}",
                            segment_first_sample,
                            self.last_sample,
                            segment_first_sample + buffer_sample_len
                        );
                        segment_first_sample
                    };
                let last_incoming_sample = first_incoming_sample + buffer_sample_len;

                if first_incoming_sample == self.last_sample {
                    // 1. append incoming buffer to the end of internal storage
                    //println!("AudioBuffer appending full incoming buffer to the end");
                    // self.first_sample unchanged
                    self.last_sample = last_incoming_sample;
                    self.last_buffer_last_sample = last_incoming_sample;

                    (
                        false,                  // first_sample_changed
                        first_incoming_sample,  // first_incoming_sample
                        0,                      // first_sample_to_add_rel
                        buffer_sample_len       // last_sample_to_add_rel
                    )
                } else if first_incoming_sample >= self.first_sample
                && last_incoming_sample <= self.last_sample {
                    // 2. incoming buffer included in current container
                    self.last_buffer_last_sample = last_incoming_sample;
                    (
                        false,                  // first_sample_changed
                        first_incoming_sample,  // first_incoming_sample
                        0,                      // first_sample_to_add_rel
                        0                       // last_sample_to_add_rel
                    )
                } else if first_incoming_sample > self.first_sample
                && first_incoming_sample < self.last_sample
                {   // 3. can append [self.last_sample, last_sample] to the end
                    // self.first_sample unchanged
                    let previous_last_sample = self.last_sample;
                    self.last_sample = last_incoming_sample;
                    // self.first_pts unchanged
                    self.last_buffer_last_sample = last_incoming_sample;
                    (
                        false,                  // first_sample_changed
                        first_incoming_sample,  // first_incoming_sample
                        previous_last_sample - segment_first_sample, // first_sample_to_add_rel
                        buffer_sample_len       // last_sample_to_add_rel
                    )
                }
                else if last_incoming_sample < self.last_sample
                && last_incoming_sample >= self.first_sample
                {   // 4. can insert [first_sample, self.first_sample] to the begining
                    let last_sample_to_add = self.first_sample;
                    self.first_sample = first_incoming_sample;
                    // self.last_sample unchanged
                    self.last_buffer_last_sample = last_incoming_sample;
                    (
                        true,                   // first_sample_changed
                        first_incoming_sample,  // first_incoming_sample
                        0,                      // first_sample_to_add_rel
                        last_sample_to_add - segment_first_sample // last_sample_to_add_rel
                    )
                } else {
                    // 5. can't merge with previous buffer
                    //println!("AudioBuffer: can't merge");
                    self.samples.clear();
                    self.first_sample = first_incoming_sample;
                    self.last_sample = last_incoming_sample;
                    self.last_buffer_last_sample = last_incoming_sample;
                    (
                        true,                   // first_sample_changed
                        first_incoming_sample,  // first_incoming_sample
                        0,                      // first_sample_to_add_rel
                        buffer_sample_len       // last_sample_to_add_rel
                    )
                }
            } else {
                // 6. initializing
                self.segement_first_sample = segment_first_sample;
                self.first_sample = segment_first_sample;
                self.last_sample = segment_first_sample + buffer_sample_len;
                self.last_buffer_last_sample = self.last_sample;
                (
                    true,                   // first_sample_changed
                    segment_first_sample,   // first_incoming_sample
                    0,                      // first_sample_to_add_rel
                    buffer_sample_len       // last_sample_to_add_rel
                )
            };

        self.last_buffer_pts = buffer_pts;

        // drain internal buffer if necessary and possible
        if !first_sample_changed
        && self.samples.len() + last_sample_to_add_rel - first_sample_to_add_rel
            > self.capacity
        {   // don't drain if samples are to be added at the begining...
            // drain only if we have enough samples in history
            // TODO: it could be worth testing truncate instead
            // (this would require reversing the buffer alimentation
            // and iteration)

            // Don't drain samples if they might be used by the extractor
            // (limit known as argument first_sample_to_keep)
            if first_sample_to_keep.min(first_incoming_sample)
                > self.first_sample + self.drain_size
            {
                //println!("draining... len before: {}", self.samples.len());
                self.samples.drain(..self.drain_size);
                self.first_sample += self.drain_size;
            }
        }

        #[cfg(feature = "profiling-audio-buffer")]
        let before_storage = Utc::now();

        // normalize samples in range 0f64..SAMPLES_NORM ready to render
        // TODO: this depends on the actual context (Waveform rendering)
        // do this in a concrete implementation

        // FIXME: use gstreamer downmix
        // FIXME: select the channels using the position info
        // if more than 2 channels,
        // Use 75% for first 2 channels (assuming front left and front right)
        // Use 25% for the others
        if last_sample_to_add_rel > 0 {
            let (front_norm_factor, others_norm_factor, front_channels) =
                if self.channels > 2 {
                    (
                        0.75f64 / 2f64 / f64::from(i16::MAX) * SAMPLES_NORM / 2f64,
                        0.25f64 / ((self.channels - 2) as f64) / f64::from(i16::MAX) * SAMPLES_NORM / 2f64,
                        2
                    )
                } else {
                    (
                        1f64 / (self.channels as f64) / f64::from(i16::MAX) * SAMPLES_OFFSET,
                        0f64,
                        self.channels
                    )
                };

            // Update container using the conditions identified above
            if !first_sample_changed || self.samples.is_empty()
            {   // samples can be push back to the container
                let mut norm_sample;
                let mut index = first_sample_to_add_rel * self.channels;
                let last = last_sample_to_add_rel * self.channels;
                while index < last {
                    norm_sample = 0f64;

                    for _ in 0..front_channels {
                        norm_sample += f64::from(incoming_samples[index]) * front_norm_factor;
                        index += 1;
                    }
                    for _ in front_channels..self.channels {
                        norm_sample += f64::from(incoming_samples[index]) * others_norm_factor;
                        index += 1;
                    }
                    self.samples.push_back(SAMPLES_OFFSET - norm_sample);
                };
            } else
            {   // samples must be inserted at the begining
                // => push front in reverse order
                let mut norm_sample;
                let mut index = last_sample_to_add_rel * self.channels;
                let first = first_sample_to_add_rel * self.channels;
                while index > first {
                    norm_sample = 0f64;

                    for _ in front_channels..self.channels {
                        index -= 1;
                        norm_sample += f64::from(incoming_samples[index]) * others_norm_factor;
                    }
                    for _ in 0..front_channels {
                        index -= 1;
                        norm_sample += f64::from(incoming_samples[index]) * front_norm_factor;
                    }
                    self.samples.push_front(SAMPLES_OFFSET - norm_sample);
                };
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

    pub fn iter(&self, first: usize, last: usize, step: usize) -> Iter {
        /*if first < self.first_sample {
            println!("iter {}, {}", first, self.first_sample);
        }*/
        assert!(first >= self.first_sample);
        let last = if last > first { last } else { first };
        Iter::new(self, first, last, step)
    }

    #[cfg(test)]
    pub fn push_samples(&mut self,
        samples: &[i16],
        first_sample: usize,
        segment_first_sample: usize,
        caps: &gst::Caps,
    ) {
        let mut samples_u8 = Vec::with_capacity(samples.len() * 2);
        let mut buf_u8 = [0; 2];
        for &sample_i16 in samples {
            LittleEndian::write_i16(&mut buf_u8, sample_i16);

            samples_u8.push(buf_u8[0]);
            samples_u8.push(buf_u8[1]);
        };

        let mut buffer = gst::Buffer::from_vec(samples_u8).unwrap();
        buffer.get_mut().unwrap().set_pts(
            (self.sample_duration * first_sample as f64) as u64 + 1
        );

        let mut segment = gst::Segment::new();
        segment.set_start(
            (self.sample_duration * segment_first_sample as f64) as u64 + 1
        );

        let self_first_sample = self.first_sample;
        self.push_gst_sample(
            gst::Sample::new(Some(buffer), Some(caps.clone()), Some(&segment), None),
            self_first_sample // never drain buffer in this test
        );
    }
}

pub struct Iter<'a> {
    buffer: &'a AudioBuffer,
    idx: usize,
    last: usize,
    step: usize,
}

impl<'a> Iter<'a> {
    fn new(buffer: &'a AudioBuffer, first: usize, last: usize, step: usize) -> Iter<'a> {
        Iter {
            buffer: buffer,
            idx: first - buffer.first_sample,
            last: buffer.samples.len().min(last - buffer.first_sample),
            step: step,
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a f64;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.last {
            return None;
        }

        let item = self.buffer.samples.get(self.idx);
        self.idx += self.step;

        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.idx >= self.last {
            return (0, Some(0));
        }

        let remaining = (self.last - self.idx) / self.step;

        (remaining, Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    extern crate gstreamer as gst;
    extern crate gstreamer_audio as gst_audio;

    use std::{i16, u16};

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
                ("channels", &1),
                ("rate", &SAMPLE_RATE),
            ],
        );
        audio_buffer.set_caps(&caps);

        // Build a buffer in the specified range
        // which would be rendered as a diagonal on a Waveform image
        // from left top corner to right bottom of the target image
        // if all samples are rendered in the range [0:SAMPLE_RATE]
        fn build_buffer(first_sample: usize, last_sample: usize) -> Vec<i16> {
            let mut buffer: Vec<i16> = Vec::new();
            let mut index = first_sample;
            while index < last_sample {
                buffer.push((
                    i16::MAX as i32
                    - (index as f64 / SAMPLE_RATE as f64 * u16::MAX as f64
                    ) as i32
                ) as i16);
                index += 1;
            }
            buffer
        }

        // samples [100:200]
        audio_buffer.push_samples(&build_buffer(100, 200), 100, 100, &caps);
        assert_eq!(audio_buffer.first_sample, 100);
        assert_eq!(audio_buffer.last_sample, 200);

        // samples [50:100]: appending to the begining
        audio_buffer.push_samples(&build_buffer(50, 100), 50, 50, &caps);
        assert_eq!(audio_buffer.first_sample, 50);
        assert_eq!(audio_buffer.last_sample, 200);

        // samples [0:75]: overlaping on the begining
        audio_buffer.push_samples(&build_buffer(0, 75), 0, 0, &caps);
        assert_eq!(audio_buffer.first_sample, 0);
        assert_eq!(audio_buffer.last_sample, 200);

        // samples [200:300]: appending to the end
        // different segment than previous
        audio_buffer.push_samples(&build_buffer(200, 300), 200, 200, &caps);
        assert_eq!(audio_buffer.first_sample, 0);
        assert_eq!(audio_buffer.last_sample, 300);

        // samples [250:400]: overlaping on the end
        audio_buffer.push_samples(&build_buffer(250, 400), 250, 250, &caps);
        assert_eq!(audio_buffer.first_sample, 0);
        assert_eq!(audio_buffer.last_sample, 400);

        // samples [400:450]: appending to the end
        // same segment as previous
        audio_buffer.push_samples(&build_buffer(400, 450), 400, 250, &caps);
        assert_eq!(audio_buffer.first_sample, 0);
        assert_eq!(audio_buffer.last_sample, 450);
    }
}
