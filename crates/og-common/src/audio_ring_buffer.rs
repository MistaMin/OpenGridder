//! Port of `Common/Source/AudioRingBuffer.hpp`.
//! Ring buffer used for plugin & processor bypass latency compensation.

#[derive(Debug, Clone)]
pub struct AudioRingBuffer<T: Copy + Default> {
    channels: usize,
    samples: usize,
    read_offset: usize,
    write_offset: usize,
    buffer: Vec<Vec<T>>,
}

impl<T: Copy + Default> Default for AudioRingBuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + Default> AudioRingBuffer<T> {
    pub fn new() -> Self {
        Self {
            channels: 0,
            samples: 0,
            read_offset: 0,
            write_offset: 0,
            buffer: Vec::new(),
        }
    }

    pub fn with_capacity(num_channels: usize, num_samples: usize, clear_new_data: bool) -> Self {
        let mut rb = Self::new();
        rb.resize(num_channels, num_samples, clear_new_data);
        rb
    }

    pub fn resize(&mut self, num_channels: usize, num_samples: usize, clear_new_data: bool) {
        self.channels = num_channels;
        self.samples = num_samples;
        self.read_offset = 0;
        self.write_offset = 0;
        self.allocate(clear_new_data);
    }

    fn allocate(&mut self, clear_new_data: bool) {
        if self.channels > 0 && self.samples > 0 {
            if self.buffer.len() != self.channels {
                self.buffer.resize_with(self.channels, Vec::new);
            }
            for ch in 0..self.channels {
                if self.buffer[ch].len() != self.samples {
                    self.buffer[ch].resize(self.samples, T::default());
                } else if clear_new_data {
                    self.buffer[ch].fill(T::default());
                }
            }
        }
    }

    pub fn clear(&mut self) {
        for ch in &mut self.buffer {
            ch.fill(T::default());
        }
    }

    pub fn get_num_channels(&self) -> usize {
        self.channels
    }

    pub fn get_num_samples(&self) -> usize {
        self.samples
    }

    pub fn set_read_offset(&mut self, offset: usize) {
        if self.samples > 0 {
            self.read_offset = offset % self.samples;
        }
    }

    pub fn inc_read_offset(&mut self, offset_to_add: usize) {
        if self.samples > 0 {
            self.read_offset = (self.read_offset + offset_to_add) % self.samples;
        }
    }

    pub fn set_write_offset(&mut self, offset: usize) {
        if self.samples > 0 {
            self.write_offset = offset % self.samples;
        }
    }

    pub fn inc_write_offset(&mut self, offset_to_add: usize) {
        if self.samples > 0 {
            self.write_offset = (self.write_offset + offset_to_add) % self.samples;
        }
    }

    pub fn read(&mut self, dst: &mut [Vec<T>], dst_start_sample: usize, num_samples: usize) -> usize {
        if self.samples == 0 {
            return 0;
        }
        let samples_to_read = self.samples.min(num_samples);
        let ch_count = self.channels.min(dst.len());
        if self.read_offset + samples_to_read <= self.samples {
            for c in 0..ch_count {
                let src_slice = &self.buffer[c][self.read_offset..self.read_offset + samples_to_read];
                dst[c][dst_start_sample..dst_start_sample + samples_to_read].copy_from_slice(src_slice);
            }
            self.inc_read_offset(samples_to_read);
        } else {
            let part1 = self.samples - self.read_offset;
            for c in 0..ch_count {
                let src_slice = &self.buffer[c][self.read_offset..self.read_offset + part1];
                dst[c][dst_start_sample..dst_start_sample + part1].copy_from_slice(src_slice);
            }
            self.inc_read_offset(part1);

            let part2 = samples_to_read - part1;
            for c in 0..ch_count {
                let src_slice = &self.buffer[c][self.read_offset..self.read_offset + part2];
                dst[c][dst_start_sample + part1..dst_start_sample + part1 + part2].copy_from_slice(src_slice);
            }
            self.inc_read_offset(part2);
        }
        samples_to_read
    }

    pub fn write(&mut self, src: &[Vec<T>], src_start_sample: usize, num_samples: usize) -> usize {
        if self.samples == 0 {
            return 0;
        }
        let samples_to_write = self.samples.min(num_samples);
        let ch_count = self.channels.min(src.len());
        if self.write_offset + samples_to_write <= self.samples {
            for c in 0..ch_count {
                let src_slice = &src[c][src_start_sample..src_start_sample + samples_to_write];
                self.buffer[c][self.write_offset..self.write_offset + samples_to_write].copy_from_slice(src_slice);
            }
            self.inc_write_offset(samples_to_write);
        } else {
            let part1 = self.samples - self.write_offset;
            for c in 0..ch_count {
                let src_slice = &src[c][src_start_sample..src_start_sample + part1];
                self.buffer[c][self.write_offset..self.write_offset + part1].copy_from_slice(src_slice);
            }
            self.inc_write_offset(part1);

            let part2 = samples_to_write - part1;
            for c in 0..ch_count {
                let src_slice = &src[c][src_start_sample + part1..src_start_sample + part1 + part2];
                self.buffer[c][self.write_offset..self.write_offset + part2].copy_from_slice(src_slice);
            }
            self.inc_write_offset(part2);
        }
        samples_to_write
    }

    pub fn process(&mut self, io_channels: &mut [Vec<T>], mut num_samples: usize) {
        if self.samples < 2 {
            return;
        }
        let mut offset = 0;
        let half = (self.samples / 2).max(1);
        while num_samples > 0 {
            let step = num_samples.min(half);
            self.write(io_channels, offset, step);
            self.read(io_channels, offset, step);
            offset += step;
            num_samples -= step;
        }
    }

    pub fn channel_data(&self, c: usize) -> &[T] {
        &self.buffer[c]
    }

    pub fn channel_data_mut(&mut self, c: usize) -> &mut [T] {
        &mut self.buffer[c]
    }
}
