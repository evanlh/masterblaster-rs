//! Time representation with sub-tick precision.

/// Time position in the song.
///
/// Uses ticks as the base unit but supports sub-tick offsets for
/// MIDI precision, swing, and humanization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    /// Absolute tick from song start
    pub tick: u64,
    /// 0-65535 subdivision within the tick (for MIDI/swing/humanize)
    pub subtick: u16,
}

impl Timestamp {
    /// Create a timestamp at an exact tick boundary.
    pub const fn from_ticks(tick: u64) -> Self {
        Self { tick, subtick: 0 }
    }

    /// Create a timestamp with a fractional offset within a tick.
    ///
    /// `fraction` should be in the range [0.0, 1.0).
    pub fn with_offset(tick: u64, fraction: f32) -> Self {
        Self {
            tick,
            subtick: (fraction.clamp(0.0, 0.99999) * 65536.0) as u16,
        }
    }

    /// Add ticks to this timestamp.
    pub const fn add_ticks(self, ticks: u64) -> Self {
        Self {
            tick: self.tick + ticks,
            subtick: self.subtick,
        }
    }

    /// Convert to a sample position given samples per tick.
    pub fn to_samples(self, samples_per_tick: u32) -> u64 {
        let base = self.tick * samples_per_tick as u64;
        let frac = (self.subtick as u64 * samples_per_tick as u64) >> 16;
        base + frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_ordering() {
        let t1 = Timestamp::from_ticks(10);
        let t2 = Timestamp::from_ticks(20);
        let t3 = Timestamp::with_offset(10, 0.5);

        assert!(t1 < t2);
        assert!(t1 < t3);
        assert!(t3 < t2);
    }

    #[test]
    fn to_samples() {
        let t = Timestamp::from_ticks(10);
        assert_eq!(t.to_samples(100), 1000);

        let t_half = Timestamp::with_offset(10, 0.5);
        assert_eq!(t_half.to_samples(100), 1050);
    }
}
