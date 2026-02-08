//! Note-to-frequency conversion for sample playback.
//!
//! Converts a MIDI note number + sample c4_speed + output sample rate
//! into a 16.16 fixed-point increment for stepping through sample data.

/// The MIDI note number that corresponds to a sample's c4_speed.
/// In MOD files, period 428 (Amiga C-2) maps to note 48 via period_to_note.
const REFERENCE_NOTE: i16 = 48;

/// Amiga period for C-4 (note 48). Used as reference in period↔frequency conversion.
const C4_PERIOD: u32 = 428;

/// Lowest allowed period (highest pitch, B-3 in Amiga notation).
pub const PERIOD_MIN: u16 = 113;

/// Highest allowed period (lowest pitch, C-1 in Amiga notation).
pub const PERIOD_MAX: u16 = 856;

/// Base periods for the lowest MOD octave (notes 36-47, C-1 to B-1 in Amiga notation).
const BASE_PERIODS: [u16; 12] = [
    856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453,
];

/// Convert a MIDI note to an Amiga period value.
///
/// Note 36 = C-1 (period 856), note 48 = C-2 (period 428), note 60 = C-3 (period 214).
/// Returns 0 for note 0 (no note).
pub fn note_to_period(note: u8) -> u16 {
    if note == 0 {
        return 0;
    }
    let offset = note as i16 - 36;
    let semitone = offset.rem_euclid(12) as usize;
    let octave = offset.div_euclid(12);
    let base = BASE_PERIODS[semitone] as u32;
    if octave >= 0 {
        (base >> octave as u32).max(1) as u16
    } else {
        (base << (-octave) as u32) as u16
    }
}

/// Convert an Amiga period + c4_speed to a 16.16 fixed-point increment.
///
/// Formula: freq = c4_speed * 428 / period, then increment = freq * 65536 / sample_rate.
pub fn period_to_increment(period: u16, c4_speed: u32, sample_rate: u32) -> u32 {
    if period == 0 || sample_rate == 0 {
        return 0;
    }
    let freq = (c4_speed as u64 * C4_PERIOD as u64) / period as u64;
    ((freq * 65536) / sample_rate as u64) as u32
}

/// Clamp a period to the valid MOD range.
pub fn clamp_period(period: u16) -> u16 {
    period.clamp(PERIOD_MIN, PERIOD_MAX)
}

/// Compute the 16.16 fixed-point sample increment for a given note.
///
/// - `note`: MIDI note number (e.g. 48 = C-4 in our system)
/// - `c4_speed`: sample's playback rate at the reference note (typically 8363 Hz)
/// - `sample_rate`: output sample rate (e.g. 44100 Hz)
pub fn note_to_increment(note: u8, c4_speed: u32, sample_rate: u32) -> u32 {
    if sample_rate == 0 || c4_speed == 0 {
        return 0;
    }

    let semitone_offset = note as i16 - REFERENCE_NOTE;
    let freq = shift_frequency(c4_speed, semitone_offset);

    // increment = freq * 65536 / sample_rate
    ((freq as u64 * 65536) / sample_rate as u64) as u32
}

/// Shift a frequency by a number of semitones using 12-TET.
/// Positive = higher pitch, negative = lower pitch.
fn shift_frequency(base_freq: u32, semitones: i16) -> u32 {
    // 2^(1/12) ≈ 1.059463
    // Use a lookup table for the fractional octave part (0-11 semitones),
    // then shift by whole octaves with bit shifts for precision.

    let octaves = semitones.div_euclid(12);
    let remainder = semitones.rem_euclid(12) as usize;

    // Multipliers for 0-11 semitones, scaled by 65536 (16.16 fixed-point)
    // semitone_multiplier[n] = round(2^(n/12) * 65536)
    const SEMITONE_MUL: [u32; 12] = [
        65536, // 0:  1.0
        69433, // 1:  2^(1/12)
        73562, // 2:  2^(2/12)
        77936, // 3:  2^(3/12)
        82570, // 4:  2^(4/12)
        87480, // 5:  2^(5/12)
        92682, // 6:  2^(6/12)
        98193, // 7:  2^(7/12)
        104032, // 8: 2^(8/12)
        110218, // 9: 2^(9/12)
        116772, // 10: 2^(10/12)
        123715, // 11: 2^(11/12)
    ];

    // freq = base_freq * 2^octaves * semitone_multiplier / 65536
    let scaled = base_freq as u64 * SEMITONE_MUL[remainder] as u64;
    let freq = scaled >> 16; // divide by 65536

    if octaves >= 0 {
        (freq << octaves as u32) as u32
    } else {
        (freq >> (-octaves) as u32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C4_SPEED: u32 = 8363; // Standard Amiga rate
    const SAMPLE_RATE: u32 = 44100;

    #[test]
    fn reference_note_gives_base_frequency() {
        // Note 48 at c4_speed 8363 should produce freq 8363
        // increment = 8363 * 65536 / 44100 ≈ 12431
        let inc = note_to_increment(48, C4_SPEED, SAMPLE_RATE);
        let expected = (C4_SPEED as u64 * 65536 / SAMPLE_RATE as u64) as u32;
        assert_eq!(inc, expected);
    }

    #[test]
    fn octave_up_doubles_increment() {
        let base = note_to_increment(48, C4_SPEED, SAMPLE_RATE);
        let octave_up = note_to_increment(60, C4_SPEED, SAMPLE_RATE);
        // Should be exactly 2x (octave = 12 semitones)
        assert_eq!(octave_up, base * 2);
    }

    #[test]
    fn octave_down_halves_increment() {
        let base = note_to_increment(48, C4_SPEED, SAMPLE_RATE);
        let octave_down = note_to_increment(36, C4_SPEED, SAMPLE_RATE);
        // Allow ±1 for fixed-point rounding on right-shift
        assert!((octave_down as i64 - base as i64 / 2).unsigned_abs() <= 1);
    }

    #[test]
    fn two_octaves_up_quadruples() {
        let base = note_to_increment(48, C4_SPEED, SAMPLE_RATE);
        let two_up = note_to_increment(72, C4_SPEED, SAMPLE_RATE);
        assert_eq!(two_up, base * 4);
    }

    #[test]
    fn semitone_up_increases_by_twelfth_root_of_two() {
        let base = note_to_increment(48, C4_SPEED, SAMPLE_RATE);
        let one_up = note_to_increment(49, C4_SPEED, SAMPLE_RATE);
        // 2^(1/12) ≈ 1.05946
        // Allow ±1 for fixed-point rounding
        let expected = (base as f64 * 1.059463) as u32;
        assert!((one_up as i64 - expected as i64).unsigned_abs() <= 1);
    }

    #[test]
    fn increment_is_nonzero_for_valid_inputs() {
        // Even very low notes should produce a nonzero increment
        let inc = note_to_increment(12, C4_SPEED, SAMPLE_RATE);
        assert!(inc > 0);
    }

    #[test]
    fn zero_sample_rate_returns_zero() {
        assert_eq!(note_to_increment(48, C4_SPEED, 0), 0);
    }

    #[test]
    fn zero_c4_speed_returns_zero() {
        assert_eq!(note_to_increment(48, 0, SAMPLE_RATE), 0);
    }

    #[test]
    fn different_c4_speed_scales_proportionally() {
        let inc_8363 = note_to_increment(48, 8363, SAMPLE_RATE);
        let inc_16726 = note_to_increment(48, 16726, SAMPLE_RATE);
        // Double c4_speed should give double increment
        assert!((inc_16726 as i64 - inc_8363 as i64 * 2).unsigned_abs() <= 1);
    }

    #[test]
    fn different_sample_rate_scales_inversely() {
        let inc_44100 = note_to_increment(48, C4_SPEED, 44100);
        let inc_22050 = note_to_increment(48, C4_SPEED, 22050);
        // Half sample rate should give double increment
        assert_eq!(inc_22050, inc_44100 * 2);
    }

    // === Period-based tests ===

    #[test]
    fn note_to_period_c1() {
        assert_eq!(note_to_period(36), 856); // C-1
    }

    #[test]
    fn note_to_period_c2() {
        assert_eq!(note_to_period(48), 428); // C-2 (half of 856)
    }

    #[test]
    fn note_to_period_c3() {
        assert_eq!(note_to_period(60), 214); // C-3 (quarter of 856)
    }

    #[test]
    fn note_to_period_b3() {
        assert_eq!(note_to_period(71), 113); // B-3 = PERIOD_MIN
    }

    #[test]
    fn note_to_period_sharp_notes() {
        assert_eq!(note_to_period(37), 808); // C#-1
        assert_eq!(note_to_period(49), 404); // C#-2
    }

    #[test]
    fn note_to_period_zero_returns_zero() {
        assert_eq!(note_to_period(0), 0);
    }

    #[test]
    fn period_to_increment_at_c4() {
        // period 428, c4_speed 8363 → freq 8363 → same as note_to_increment(48, ...)
        let inc = period_to_increment(428, C4_SPEED, SAMPLE_RATE);
        let expected = note_to_increment(48, C4_SPEED, SAMPLE_RATE);
        assert_eq!(inc, expected);
    }

    #[test]
    fn period_to_increment_octave_up_doubles() {
        let base = period_to_increment(428, C4_SPEED, SAMPLE_RATE);
        let octave_up = period_to_increment(214, C4_SPEED, SAMPLE_RATE);
        assert_eq!(octave_up, base * 2);
    }

    #[test]
    fn period_to_increment_zero_period_returns_zero() {
        assert_eq!(period_to_increment(0, C4_SPEED, SAMPLE_RATE), 0);
    }

    #[test]
    fn period_to_increment_zero_sample_rate_returns_zero() {
        assert_eq!(period_to_increment(428, C4_SPEED, 0), 0);
    }

    #[test]
    fn note_to_period_roundtrip_matches_increment() {
        // For note 48 (C-2), note_to_period → period_to_increment should match
        let period = note_to_period(48);
        let via_period = period_to_increment(period, C4_SPEED, SAMPLE_RATE);
        let via_note = note_to_increment(48, C4_SPEED, SAMPLE_RATE);
        assert_eq!(via_period, via_note);
    }

    #[test]
    fn clamp_period_within_range() {
        assert_eq!(clamp_period(428), 428);
    }

    #[test]
    fn clamp_period_below_min() {
        assert_eq!(clamp_period(50), PERIOD_MIN);
    }

    #[test]
    fn clamp_period_above_max() {
        assert_eq!(clamp_period(1000), PERIOD_MAX);
    }
}
