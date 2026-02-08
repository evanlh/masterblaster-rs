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
