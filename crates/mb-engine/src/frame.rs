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

    /// Mix another frame into this one.
    pub fn mix(&mut self, other: Frame) {
        // Use i32 to avoid overflow, then clamp
        let left = (self.left as i32 + other.left as i32).clamp(-32768, 32767);
        let right = (self.right as i32 + other.right as i32).clamp(-32768, 32767);
        self.left = left as i16;
        self.right = right as i16;
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

    /// Accumulate a narrow frame with wire gain applied.
    ///
    /// `gain` is stored as `(ratio * 100 - 100)` where 0 = unity.
    /// We convert back: `linear = (gain + 100) / 100`.
    pub fn accumulate_with_gain(&mut self, frame: Frame, gain: i16) {
        if gain == 0 {
            self.accumulate(frame);
        } else {
            let scale = (gain as i32) + 100;
            self.left += (frame.left as i32 * scale) / 100;
            self.right += (frame.right as i32 * scale) / 100;
        }
    }

    /// Attenuate and clamp to i16 Frame.
    pub fn to_frame(self, shift: u32) -> Frame {
        Frame {
            left: (self.left >> shift).clamp(-32768, 32767) as i16,
            right: (self.right >> shift).clamp(-32768, 32767) as i16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulate_with_gain_unity() {
        let mut wide = WideFrame::silence();
        wide.accumulate_with_gain(Frame { left: 1000, right: -500 }, 0);
        assert_eq!(wide.left, 1000);
        assert_eq!(wide.right, -500);
    }

    #[test]
    fn accumulate_with_gain_half() {
        let mut wide = WideFrame::silence();
        // gain = -50 → scale = 50 → 50/100 = 0.5x
        wide.accumulate_with_gain(Frame { left: 1000, right: -1000 }, -50);
        assert_eq!(wide.left, 500);
        assert_eq!(wide.right, -500);
    }

    #[test]
    fn accumulate_with_gain_muted() {
        let mut wide = WideFrame::silence();
        // gain = -100 → scale = 0 → muted
        wide.accumulate_with_gain(Frame { left: 1000, right: 1000 }, -100);
        assert_eq!(wide.left, 0);
        assert_eq!(wide.right, 0);
    }
}
