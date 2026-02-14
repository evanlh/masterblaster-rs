//! Beat-based time representation.
//!
//! `MusicalTime` uses beats as the universal time coordinate.
//! Trackers, MIDI, and DAWs all understand beats, making this
//! a format-agnostic position type.

/// Subdivisions per beat. LCM(1..16) = 720720, divisible by
/// any rows_per_beat value from 1 to 16.
pub const SUB_BEAT_UNIT: u32 = 720_720;

/// A position in musical time (beats + fractional sub-beat).
///
/// Ordering: beat is primary, sub_beat is secondary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MusicalTime {
    /// Whole beats from song start
    pub beat: u64,
    /// Fraction of a beat: 0..SUB_BEAT_UNIT
    pub sub_beat: u32,
}

impl MusicalTime {
    /// The zero position (song start).
    pub const fn zero() -> Self {
        Self { beat: 0, sub_beat: 0 }
    }

    /// Create a time at an exact beat boundary.
    pub const fn from_beats(beat: u64) -> Self {
        Self { beat, sub_beat: 0 }
    }

    /// Advance by `rows` rows at `rows_per_beat` resolution.
    pub fn add_rows(self, rows: u32, rows_per_beat: u32) -> Self {
        let sub_per_row = SUB_BEAT_UNIT / rows_per_beat;
        let total_sub = self.sub_beat as u64 + rows as u64 * sub_per_row as u64;
        let extra_beats = total_sub / SUB_BEAT_UNIT as u64;
        let remaining = (total_sub % SUB_BEAT_UNIT as u64) as u32;
        Self {
            beat: self.beat + extra_beats,
            sub_beat: remaining,
        }
    }

    /// Advance by `ticks` ticks at `ticks_per_beat` resolution.
    /// Used for NoteDelay sub-beat offsets.
    pub fn add_ticks(self, ticks: u32, ticks_per_beat: u32) -> Self {
        if ticks_per_beat == 0 {
            return self;
        }
        let sub_per_tick = SUB_BEAT_UNIT / ticks_per_beat;
        let total_sub = self.sub_beat as u64 + ticks as u64 * sub_per_tick as u64;
        let extra_beats = total_sub / SUB_BEAT_UNIT as u64;
        let remaining = (total_sub % SUB_BEAT_UNIT as u64) as u32;
        Self {
            beat: self.beat + extra_beats,
            sub_beat: remaining,
        }
    }
}

impl PartialOrd for MusicalTime {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MusicalTime {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.beat.cmp(&other.beat).then(self.sub_beat.cmp(&other.sub_beat))
    }
}

/// Pack a MusicalTime into a u64: (beat as u32) << 32 | sub_beat.
pub fn pack_time(t: MusicalTime) -> u64 {
    ((t.beat as u32 as u64) << 32) | t.sub_beat as u64
}

/// Unpack a u64 into a MusicalTime.
pub fn unpack_time(packed: u64) -> MusicalTime {
    MusicalTime {
        beat: (packed >> 32) as u64,
        sub_beat: packed as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_default() {
        assert_eq!(MusicalTime::zero(), MusicalTime::default());
    }

    #[test]
    fn from_beats_sets_sub_beat_zero() {
        let t = MusicalTime::from_beats(5);
        assert_eq!(t.beat, 5);
        assert_eq!(t.sub_beat, 0);
    }

    #[test]
    fn ordering() {
        let t0 = MusicalTime::zero();
        let t1 = MusicalTime::from_beats(1);
        let t_half = MusicalTime { beat: 0, sub_beat: SUB_BEAT_UNIT / 2 };
        assert!(t0 < t_half);
        assert!(t_half < t1);
    }

    #[test]
    fn add_rows_within_beat() {
        // 4 rows per beat: each row = 180180 sub_beats
        let t = MusicalTime::zero().add_rows(2, 4);
        assert_eq!(t.beat, 0);
        assert_eq!(t.sub_beat, 2 * (SUB_BEAT_UNIT / 4));
    }

    #[test]
    fn add_rows_crosses_beat_boundary() {
        // 4 rows/beat, add 6 rows = 1 beat + 2 rows
        let t = MusicalTime::zero().add_rows(6, 4);
        assert_eq!(t.beat, 1);
        assert_eq!(t.sub_beat, 2 * (SUB_BEAT_UNIT / 4));
    }

    #[test]
    fn add_rows_exact_beat() {
        let t = MusicalTime::zero().add_rows(4, 4);
        assert_eq!(t.beat, 1);
        assert_eq!(t.sub_beat, 0);
    }

    #[test]
    fn add_rows_from_nonzero() {
        let start = MusicalTime { beat: 2, sub_beat: SUB_BEAT_UNIT / 4 };
        let t = start.add_rows(3, 4);
        // 1 row into beat 2 + 3 rows = 4 rows total from beat 2 start = beat 3 start
        assert_eq!(t.beat, 3);
        assert_eq!(t.sub_beat, 0);
    }

    #[test]
    fn add_ticks_basic() {
        // 24 ticks/beat (speed 6, rpb 4): sub_per_tick = 720720/24 = 30030
        let t = MusicalTime::zero().add_ticks(3, 24);
        assert_eq!(t.sub_beat, 3 * (SUB_BEAT_UNIT / 24));
    }

    #[test]
    fn add_ticks_zero_tpb_is_noop() {
        let t = MusicalTime::from_beats(5);
        assert_eq!(t.add_ticks(10, 0), t);
    }

    #[test]
    fn sub_beat_unit_divisibility() {
        // SUB_BEAT_UNIT should be evenly divisible by 1..16
        for n in 1..=16 {
            assert_eq!(
                SUB_BEAT_UNIT % n, 0,
                "SUB_BEAT_UNIT not divisible by {}", n
            );
        }
    }
}
