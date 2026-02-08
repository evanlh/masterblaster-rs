//! Sample data types.

use alloc::vec::Vec;
use arrayvec::ArrayString;

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

    /// Returns true if stereo.
    pub fn is_stereo(&self) -> bool {
        matches!(self, SampleData::Stereo8(..) | SampleData::Stereo16(..))
    }

    /// Returns true if 16-bit.
    pub fn is_16bit(&self) -> bool {
        matches!(self, SampleData::Mono16(_) | SampleData::Stereo16(..))
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

    /// Get stereo sample values at position (as i16, i16).
    pub fn get_stereo(&self, pos: usize) -> (i16, i16) {
        match self {
            SampleData::Mono8(v) => {
                let s = v.get(pos).copied().unwrap_or(0) as i16 * 256;
                (s, s)
            }
            SampleData::Mono16(v) => {
                let s = v.get(pos).copied().unwrap_or(0);
                (s, s)
            }
            SampleData::Stereo8(l, r) => {
                let ls = l.get(pos).copied().unwrap_or(0) as i16 * 256;
                let rs = r.get(pos).copied().unwrap_or(0) as i16 * 256;
                (ls, rs)
            }
            SampleData::Stereo16(l, r) => {
                let ls = l.get(pos).copied().unwrap_or(0);
                let rs = r.get(pos).copied().unwrap_or(0);
                (ls, rs)
            }
        }
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
