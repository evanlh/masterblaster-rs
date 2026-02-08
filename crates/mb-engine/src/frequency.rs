//! Note-to-frequency conversion for sample playback.
//!
//! Converts a MIDI note number + sample c4_speed + output sample rate
//! into a 16.16 fixed-point increment for stepping through sample data.

/// The MIDI note number that corresponds to a sample's c4_speed.
/// In MOD files, period 428 (Amiga C-2) maps to note 48 via period_to_note.
const REFERENCE_NOTE: i16 = 48;

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
}
