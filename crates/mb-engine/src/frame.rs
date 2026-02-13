//! Audio frame type.

/// A stereo audio frame (16-bit integer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub left: i16,
    pub right: i16,
}

impl Frame {
    /// Create a silent frame.
    pub const fn silence() -> Self {
        Self { left: 0, right: 0 }
    }

    /// Create a mono frame (same value for both channels).
    pub const fn mono(value: i16) -> Self {
        Self {
            left: value,
            right: value,
        }
    }

    /// Mix another frame into this one.
    pub fn mix(&mut self, other: Frame) {
        // Use i32 to avoid overflow, then clamp
        let left = (self.left as i32 + other.left as i32).clamp(-32768, 32767);
        let right = (self.right as i32 + other.right as i32).clamp(-32768, 32767);
        self.left = left as i16;
        self.right = right as i16;
    }

    /// Apply volume (0-64 scale).
    pub fn apply_volume(&mut self, volume: u8) {
        self.left = ((self.left as i32 * volume as i32) >> 6) as i16;
        self.right = ((self.right as i32 * volume as i32) >> 6) as i16;
    }
}

/// A wide (i32) stereo frame for accumulating multiple i16 inputs without premature clamping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WideFrame {
    pub left: i32,
    pub right: i32,
}

impl WideFrame {
    pub const fn silence() -> Self {
        Self { left: 0, right: 0 }
    }

    /// Accumulate a narrow frame without clamping.
    pub fn accumulate(&mut self, frame: Frame) {
        self.left += frame.left as i32;
        self.right += frame.right as i32;
    }

    /// Attenuate and clamp to i16 Frame.
    pub fn to_frame(self, shift: u32) -> Frame {
        Frame {
            left: (self.left >> shift).clamp(-32768, 32767) as i16,
            right: (self.right >> shift).clamp(-32768, 32767) as i16,
        }
    }
}
