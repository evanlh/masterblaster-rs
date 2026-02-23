//! Multichannel f32 audio buffer with planar layout.

use alloc::vec;
use alloc::vec::Vec;

/// Maximum number of audio channels per buffer.
pub const MAX_CHANNELS: u16 = 8;

/// Default block size for audio processing.
pub const BLOCK_SIZE: usize = 256;

/// A multichannel f32 audio buffer in planar layout.
///
/// Data is stored as `channels` contiguous planes of `frames` samples each.
/// `data[ch * frames + frame]` gives the sample for channel `ch` at `frame`.
#[derive(Clone, Debug)]
pub struct AudioBuffer {
    data: Vec<f32>,
    channels: u16,
    frames: u16,
}

impl AudioBuffer {
    /// Create a new silent buffer with the given dimensions.
    pub fn new(channels: u16, frames: u16) -> Self {
        Self {
            data: vec![0.0; channels as usize * frames as usize],
            channels,
            frames,
        }
    }

    /// Fill all samples with zero.
    pub fn silence(&mut self) {
        self.data.fill(0.0);
    }

    /// Number of channels.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Number of frames.
    pub fn frames(&self) -> u16 {
        self.frames
    }

    /// Read-only access to one channel's sample data.
    pub fn channel(&self, ch: u16) -> &[f32] {
        let start = ch as usize * self.frames as usize;
        &self.data[start..start + self.frames as usize]
    }

    /// Mutable access to one channel's sample data.
    pub fn channel_mut(&mut self, ch: u16) -> &mut [f32] {
        let start = ch as usize * self.frames as usize;
        let len = self.frames as usize;
        &mut self.data[start..start + len]
    }

    /// Sum overlapping channels from `source` into this buffer.
    pub fn mix_from(&mut self, source: &AudioBuffer) {
        let chs = self.channels.min(source.channels);
        let frs = self.frames.min(source.frames) as usize;
        for ch in 0..chs {
            let dst = self.channel_mut(ch);
            let src = source.channel(ch);
            for i in 0..frs {
                dst[i] += src[i];
            }
        }
    }

    /// Sum overlapping channels from `source` into this buffer with gain.
    pub fn mix_from_scaled(&mut self, source: &AudioBuffer, gain: f32) {
        let chs = self.channels.min(source.channels);
        let frs = self.frames.min(source.frames) as usize;
        for ch in 0..chs {
            let dst = self.channel_mut(ch);
            let src = source.channel(ch);
            for i in 0..frs {
                dst[i] += src[i] * gain;
            }
        }
    }

    /// Scale all samples by `gain`.
    pub fn apply_gain(&mut self, gain: f32) {
        for s in &mut self.data {
            *s *= gain;
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
    fn apply_gain_scales_all() {
        let mut buf = AudioBuffer::new(2, 1);
        buf.channel_mut(0)[0] = 1.0;
        buf.channel_mut(1)[0] = -0.5;
        buf.apply_gain(2.0);
        assert!((buf.channel(0)[0] - 2.0).abs() < 1e-6);
        assert!((buf.channel(1)[0] - -1.0).abs() < 1e-6);
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
