//! Sample data types.

use alloc::vec::Vec;
use arrayvec::ArrayString;

slotmap::new_key_type! {
    /// Key for referencing samples in the VoicePool's sample bank.
    pub struct SampleKey;
}

/// A sample definition.
#[derive(Clone, Debug)]
pub struct Sample {
    /// Sample name
    pub name: ArrayString<26>,
    /// Audio data
    pub data: SampleData,
    /// Loop start position (in samples)
    pub loop_start: u32,
    /// Loop end position (in samples)
    pub loop_end: u32,
    /// Loop type
    pub loop_type: LoopType,
    /// Default volume (0-64)
    pub default_volume: u8,
    /// Default panning (-64 to +64, 0 = center)
    pub default_pan: i8,
    /// Frequency of C-4 in Hz (typically 8363 for MOD)
    pub c4_speed: u32,
    /// Auto-vibrato settings
    pub vibrato: Option<AutoVibrato>,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            name: ArrayString::new(),
            data: SampleData::Mono8(Vec::new()),
            loop_start: 0,
            loop_end: 0,
            loop_type: LoopType::None,
            default_volume: 64,
            default_pan: 0,
            c4_speed: 8363,
            vibrato: None,
        }
    }
}

impl Sample {
    /// Create a new empty sample.
    pub fn new(name: &str) -> Self {
        let mut sample = Self::default();
        let _ = sample.name.try_push_str(name);
        sample
    }

    /// Get the length of the sample in frames.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if the sample has no data.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns true if the sample has a loop.
    pub fn has_loop(&self) -> bool {
        self.loop_type != LoopType::None && self.loop_end > self.loop_start
    }
}

/// Sample audio data.
#[derive(Clone, Debug)]
pub enum SampleData {
    /// 8-bit mono samples
    Mono8(Vec<i8>),
    /// 16-bit mono samples
    Mono16(Vec<i16>),
    /// 8-bit stereo samples (left, right)
    Stereo8(Vec<i8>, Vec<i8>),
    /// 16-bit stereo samples (left, right)
    Stereo16(Vec<i16>, Vec<i16>),
}

impl SampleData {
    /// Get the number of sample frames.
    pub fn len(&self) -> usize {
        match self {
            SampleData::Mono8(v) => v.len(),
            SampleData::Mono16(v) => v.len(),
            SampleData::Stereo8(l, _) => l.len(),
            SampleData::Stereo16(l, _) => l.len(),
        }
    }

    /// Returns true if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get a mono sample value at position (as i16).
    /// For stereo, returns the left channel.
    pub fn get_mono(&self, pos: usize) -> i16 {
        match self {
            SampleData::Mono8(v) => v.get(pos).copied().unwrap_or(0) as i16 * 256,
            SampleData::Mono16(v) => v.get(pos).copied().unwrap_or(0),
            SampleData::Stereo8(l, _) => l.get(pos).copied().unwrap_or(0) as i16 * 256,
            SampleData::Stereo16(l, _) => l.get(pos).copied().unwrap_or(0),
        }
    }

    /// Get a linearly interpolated mono sample value.
    ///
    /// `pos` is a 16.16 fixed-point position. Blends between the two
    /// nearest sample values using the fractional part.
    pub fn get_mono_interpolated(&self, pos_fixed: u32) -> i16 {
        let idx = (pos_fixed >> 16) as usize;
        let frac = (pos_fixed & 0xFFFF) as i64;

        let a = self.get_mono(idx) as i64;
        let b = self.get_mono(idx + 1) as i64;

        (a + (((b - a) * frac) >> 16)) as i16
    }

    /// Number of channels in the sample data.
    pub fn num_channels(&self) -> u16 {
        match self {
            SampleData::Mono8(_) | SampleData::Mono16(_) => 1,
            SampleData::Stereo8(_, _) | SampleData::Stereo16(_, _) => 2,
        }
    }

    /// Get a sample from the right channel (returns left for mono).
    pub fn get_right(&self, pos: usize) -> i16 {
        match self {
            SampleData::Mono8(v) => v.get(pos).copied().unwrap_or(0) as i16 * 256,
            SampleData::Mono16(v) => v.get(pos).copied().unwrap_or(0),
            SampleData::Stereo8(_, r) => r.get(pos).copied().unwrap_or(0) as i16 * 256,
            SampleData::Stereo16(_, r) => r.get(pos).copied().unwrap_or(0),
        }
    }
}

impl crate::audio_traits::AudioSource for SampleData {
    fn channels(&self) -> u16 {
        self.num_channels()
    }

    fn frames(&self) -> usize {
        self.len()
    }

    fn read_i16(&self, ch: u16, frame: usize) -> i16 {
        if ch == 0 { self.get_mono(frame) } else { self.get_right(frame) }
    }
}

/// Sample loop type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopType {
    /// No loop
    #[default]
    None,
    /// Forward loop
    Forward,
    /// Ping-pong (bidirectional) loop
    PingPong,
    /// Sustain loop (release on note-off)
    Sustain,
}

/// Auto-vibrato settings for a sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct AutoVibrato {
    /// Vibrato speed
    pub speed: u8,
    /// Vibrato depth
    pub depth: u8,
    /// Vibrato sweep (ramp-up time)
    pub sweep: u8,
    /// Waveform type (0=sine, 1=ramp down, 2=square, 3=random)
    pub waveform: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono8_sample(data: &[i8]) -> SampleData {
        SampleData::Mono8(data.to_vec())
    }

    #[test]
    fn interpolated_at_integer_matches_nearest() {
        let data = mono8_sample(&[0, 100, -50, 30]);
        // pos_fixed = 1 << 16 = index 1, frac 0
        assert_eq!(data.get_mono_interpolated(1 << 16), data.get_mono(1));
    }

    #[test]
    fn interpolated_midpoint_averages_neighbors() {
        let data = mono8_sample(&[0, 100]);
        // Midpoint: index 0, frac = 0.5 = 32768
        let mid = data.get_mono_interpolated(32768);
        let a = data.get_mono(0) as i32; // 0
        let b = data.get_mono(1) as i32; // 25600
        let expected = ((a + b) / 2) as i16;
        assert!((mid as i32 - expected as i32).abs() <= 1);
    }

    #[test]
    fn interpolated_quarter_blends_75_25() {
        let data = mono8_sample(&[0, 100]);
        // 0.25 = 16384
        let val = data.get_mono_interpolated(16384);
        let a = data.get_mono(0) as i32;
        let b = data.get_mono(1) as i32;
        let expected = (a + (b - a) / 4) as i16;
        assert!((val as i32 - expected as i32).abs() <= 1);
    }

    #[test]
    fn interpolated_past_end_fades_to_zero() {
        let data = mono8_sample(&[100]);
        // pos at index 0, frac 0.5: blends sample[0] with sample[1] (out of bounds → 0)
        let val = data.get_mono_interpolated(32768);
        let a = data.get_mono(0) as i32; // 25600
        let expected = (a / 2) as i16;
        assert!((val as i32 - expected as i32).abs() <= 1);
    }

    // --- AudioSource impl tests ---

    use crate::audio_traits::AudioSource;

    #[test]
    fn audio_source_mono8() {
        let data = SampleData::Mono8(vec![0, 100, -50]);
        assert_eq!(AudioSource::channels(&data), 1);
        assert_eq!(AudioSource::frames(&data), 3);
        assert_eq!(data.read_i16(0, 1), 100 * 256);
        let f = data.read_f32(0, 1);
        assert!((f - 100.0 * 256.0 / 32768.0).abs() < 1e-4);
    }

    #[test]
    fn audio_source_stereo16() {
        let data = SampleData::Stereo16(vec![1000, -1000], vec![2000, -2000]);
        assert_eq!(AudioSource::channels(&data), 2);
        assert_eq!(AudioSource::frames(&data), 2);
        assert_eq!(data.read_i16(0, 0), 1000);
        assert_eq!(data.read_i16(1, 0), 2000);
    }
}
