//! Instrument and envelope types.

use alloc::vec::Vec;
use arrayvec::ArrayString;

/// An instrument definition.
#[derive(Clone, Debug)]
pub struct Instrument {
    /// Instrument name
    pub name: ArrayString<26>,
    /// Sample mapping: note (0-119) -> sample index
    pub sample_map: [u8; 120],
    /// Volume envelope
    pub volume_envelope: Option<Envelope>,
    /// Panning envelope
    pub panning_envelope: Option<Envelope>,
    /// Pitch/filter envelope (IT-specific)
    pub pitch_envelope: Option<Envelope>,
    /// Fadeout speed (0 = no fade)
    pub fadeout: u16,
    /// What happens when a new note is played on a channel already playing this instrument
    pub new_note_action: NewNoteAction,
    /// Duplicate note checking mode
    pub duplicate_check: DuplicateCheck,
}

impl Default for Instrument {
    fn default() -> Self {
        Self {
            name: ArrayString::new(),
            sample_map: [0; 120],
            volume_envelope: None,
            panning_envelope: None,
            pitch_envelope: None,
            fadeout: 0,
            new_note_action: NewNoteAction::Cut,
            duplicate_check: DuplicateCheck::Off,
        }
    }
}

impl Instrument {
    /// Create a new instrument with default settings.
    pub fn new(name: &str) -> Self {
        let mut inst = Self::default();
        let _ = inst.name.try_push_str(name);
        inst
    }

    /// Set all notes to map to a single sample.
    pub fn set_single_sample(&mut self, sample_index: u8) {
        self.sample_map.fill(sample_index);
    }
}

/// Action when a new note triggers on a channel already playing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewNoteAction {
    /// Cut the previous note immediately
    #[default]
    Cut,
    /// Continue the previous note (background)
    Continue,
    /// Send note-off to previous note
    Off,
    /// Fade out the previous note
    Fade,
}

/// Duplicate note checking mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DuplicateCheck {
    /// No duplicate checking
    #[default]
    Off,
    /// Check for duplicate notes
    Note,
    /// Check for duplicate samples
    Sample,
    /// Check for duplicate instruments
    Instrument,
}

/// An envelope (volume, panning, or pitch).
#[derive(Clone, Debug, Default)]
pub struct Envelope {
    /// Envelope points
    pub points: Vec<EnvelopePoint>,
    /// Sustain loop start point index (None = no sustain)
    pub sustain_start: Option<u8>,
    /// Sustain loop end point index
    pub sustain_end: Option<u8>,
    /// Regular loop start point index (None = no loop)
    pub loop_start: Option<u8>,
    /// Regular loop end point index
    pub loop_end: Option<u8>,
    /// Is the envelope enabled?
    pub enabled: bool,
}


impl Envelope {
    /// Create a new empty envelope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a point to the envelope.
    pub fn add_point(&mut self, tick: u16, value: i8) {
        self.points.push(EnvelopePoint { tick, value });
    }

    /// Get the interpolated value at a given tick.
    pub fn value_at(&self, tick: u16) -> i8 {
        if self.points.is_empty() {
            return 0;
        }

        // Find surrounding points
        let mut prev = &self.points[0];
        for point in &self.points {
            if point.tick > tick {
                // Interpolate between prev and point
                if point.tick == prev.tick {
                    return point.value;
                }
                let t = (tick - prev.tick) as i32;
                let d = (point.tick - prev.tick) as i32;
                let v = prev.value as i32 + (point.value as i32 - prev.value as i32) * t / d;
                return v as i8;
            }
            prev = point;
        }

        // Past the last point
        prev.value
    }
}

/// A point in an envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvelopePoint {
    /// Tick position (0-65535)
    pub tick: u16,
    /// Value (-64 to +64, or 0-64 for volume)
    pub value: i8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_interpolation() {
        let mut env = Envelope::new();
        env.add_point(0, 64);
        env.add_point(100, 0);

        assert_eq!(env.value_at(0), 64);
        assert_eq!(env.value_at(50), 32);
        assert_eq!(env.value_at(100), 0);
        assert_eq!(env.value_at(200), 0); // Past end
    }
}
