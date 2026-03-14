//! Multichannel f32 audio buffer with planar layout.

use alloc::vec;
use alloc::vec::Vec;

/// Maximum number of audio channels per buffer.
pub const MAX_CHANNELS: u16 = 8;

/// Default block size for audio processing.
pub const BLOCK_SIZE: usize = 256;

/// A multichannel f32 audio buffer in planar layout.
///
/// Data is stored as `channels` contiguous planes of `capacity` samples each.
/// `data[ch * capacity + frame]` gives the sample for channel `ch` at `frame`.
/// `frames` represents the active sub-range (≤ capacity) for rendering.
#[derive(Clone, Debug)]
pub struct AudioBuffer {
    data: Vec<f32>,
    channels: u16,
    /// Allocated frames per channel (determines memory layout / channel stride).
    capacity: u16,
    /// Active frame count for rendering (≤ capacity).
    frames: u16,
}

impl AudioBuffer {
    /// Create a new silent buffer with the given dimensions.
    pub fn new(channels: u16, frames: u16) -> Self {
        Self {
            data: vec![0.0; channels as usize * frames as usize],
            channels,
            capacity: frames,
            frames,
        }
    }

    /// Fill all active frames with zero.
    pub fn silence(&mut self) {
        for ch in 0..self.channels {
            let start = ch as usize * self.capacity as usize;
            self.data[start..start + self.frames as usize].fill(0.0);
        }
    }

    /// Set the active frame count (must not exceed allocated capacity).
    /// This allows rendering sub-blocks without reallocating.
    pub fn set_frames(&mut self, frames: u16) {
        debug_assert!(
            frames <= self.capacity,
            "set_frames({}) exceeds capacity ({})",
            frames, self.capacity,
        );
        self.frames = frames;
    }

    /// Number of channels.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Number of frames.
    pub fn frames(&self) -> u16 {
        self.frames
    }

    /// Read-only access to one channel's active sample data.
    #[inline(always)]
    pub fn channel(&self, ch: u16) -> &[f32] {
        let start = ch as usize * self.capacity as usize;
        &self.data[start..start + self.frames as usize]
    }

    /// Mutable access to one channel's active sample data.
    #[inline(always)]
    pub fn channel_mut(&mut self, ch: u16) -> &mut [f32] {
        let start = ch as usize * self.capacity as usize;
        let len = self.frames as usize;
        &mut self.data[start..start + len]
    }

    /// Get mutable access to two different channels simultaneously.
    /// Panics if `ch_a == ch_b` or either is out of range.
    #[inline(always)]
    pub fn channels_mut_2(&mut self, ch_a: u16, ch_b: u16) -> (&mut [f32], &mut [f32]) {
        assert_ne!(ch_a, ch_b, "channels must differ");
        let stride = self.capacity as usize;
        let len = self.frames as usize;
        let a_start = ch_a as usize * stride;
        let b_start = ch_b as usize * stride;
        if a_start < b_start {
            let (first, rest) = self.data.split_at_mut(b_start);
            (&mut first[a_start..a_start + len], &mut rest[..len])
        } else {
            let (first, rest) = self.data.split_at_mut(a_start);
            (&mut rest[..len], &mut first[b_start..b_start + len])
        }
    }

    /// Sum overlapping channels from `source` into this buffer.
    #[inline(always)]
    pub fn mix_from(&mut self, source: &AudioBuffer) {
        let chs = self.channels.min(source.channels);
        // channel() already returns active-frames-sized slices
        for ch in 0..chs {
            let frs = self.channel(ch).len().min(source.channel(ch).len());
            let src_start = ch as usize * source.capacity as usize;
            let dst_start = ch as usize * self.capacity as usize;
            for i in 0..frs {
                self.data[dst_start + i] += source.data[src_start + i];
            }
        }
    }

    /// Sum overlapping channels from `source` into this buffer with gain.
    #[inline(always)]
    pub fn mix_from_scaled(&mut self, source: &AudioBuffer, gain: f32) {
        let chs = self.channels.min(source.channels);
        for ch in 0..chs {
            let frs = self.channel(ch).len().min(source.channel(ch).len());
            let src_start = ch as usize * source.capacity as usize;
            let dst_start = ch as usize * self.capacity as usize;
            for i in 0..frs {
                self.data[dst_start + i] += source.data[src_start + i] * gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_silent() {
        let buf = AudioBuffer::new(2, 4);
        assert_eq!(buf.channels(), 2);
        assert_eq!(buf.frames(), 4);
        assert!(buf.channel(0).iter().all(|&s| s == 0.0));
        assert!(buf.channel(1).iter().all(|&s| s == 0.0));
    }

    #[test]
    fn channel_mut_writes_correctly() {
        let mut buf = AudioBuffer::new(2, 2);
        buf.channel_mut(0)[0] = 1.0;
        buf.channel_mut(1)[1] = -0.5;
        assert_eq!(buf.channel(0), &[1.0, 0.0]);
        assert_eq!(buf.channel(1), &[0.0, -0.5]);
    }

    #[test]
    fn silence_clears_data() {
        let mut buf = AudioBuffer::new(1, 2);
        buf.channel_mut(0)[0] = 1.0;
        buf.silence();
        assert_eq!(buf.channel(0), &[0.0, 0.0]);
    }

    #[test]
    fn mix_from_sums_channels() {
        let mut dst = AudioBuffer::new(2, 2);
        dst.channel_mut(0)[0] = 0.5;

        let mut src = AudioBuffer::new(2, 2);
        src.channel_mut(0)[0] = 0.3;
        src.channel_mut(1)[1] = 0.7;

        dst.mix_from(&src);
        assert!((dst.channel(0)[0] - 0.8).abs() < 1e-6);
        assert!((dst.channel(1)[1] - 0.7).abs() < 1e-6);
    }

    #[test]
    fn mix_from_scaled_applies_gain() {
        let mut dst = AudioBuffer::new(1, 2);
        let mut src = AudioBuffer::new(1, 2);
        src.channel_mut(0)[0] = 1.0;
        src.channel_mut(0)[1] = -1.0;

        dst.mix_from_scaled(&src, 0.5);
        assert!((dst.channel(0)[0] - 0.5).abs() < 1e-6);
        assert!((dst.channel(0)[1] - -0.5).abs() < 1e-6);
    }

    #[test]
    fn mix_from_mismatched_sizes_uses_minimum() {
        let mut dst = AudioBuffer::new(2, 4);
        let mut src = AudioBuffer::new(1, 2);
        src.channel_mut(0)[0] = 1.0;
        src.channel_mut(0)[1] = 2.0;

        dst.mix_from(&src);
        // Only channel 0, frames 0-1 mixed
        assert!((dst.channel(0)[0] - 1.0).abs() < 1e-6);
        assert!((dst.channel(0)[1] - 2.0).abs() < 1e-6);
        assert_eq!(dst.channel(0)[2], 0.0);
        assert_eq!(dst.channel(1)[0], 0.0);
    }
}
