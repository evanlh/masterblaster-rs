//! Pattern and cell types for tracker sequences.

use alloc::vec::Vec;
use crate::effects::{Effect, VolumeCommand};

/// A note value in a pattern cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Note {
    /// No note
    #[default]
    None,
    /// Note on with MIDI note number (0-119, where 60 = C-4)
    On(u8),
    /// Note off / key release
    Off,
    /// Note fade (IT-specific)
    Fade,
}

impl Note {
    /// Create a note from octave (0-9) and semitone (0-11).
    pub const fn from_octave_semitone(octave: u8, semitone: u8) -> Self {
        Note::On(octave * 12 + semitone)
    }

    /// Get the octave (0-9) if this is a note on.
    pub const fn octave(self) -> Option<u8> {
        match self {
            Note::On(n) => Some(n / 12),
            _ => None,
        }
    }

    /// Get the semitone (0-11) if this is a note on.
    pub const fn semitone(self) -> Option<u8> {
        match self {
            Note::On(n) => Some(n % 12),
            _ => None,
        }
    }
}

/// A single cell in a pattern.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// Note value
    pub note: Note,
    /// Instrument number (0 = none, 1-255 = instrument index + 1)
    pub instrument: u8,
    /// Volume column command
    pub volume: VolumeCommand,
    /// Effect column command
    pub effect: Effect,
}

impl Cell {
    /// Create an empty cell.
    pub const fn empty() -> Self {
        Self {
            note: Note::None,
            instrument: 0,
            volume: VolumeCommand::None,
            effect: Effect::None,
        }
    }

    /// Returns true if the cell is completely empty.
    pub fn is_empty(&self) -> bool {
        self.note == Note::None
            && self.instrument == 0
            && self.volume == VolumeCommand::None
            && self.effect == Effect::None
    }
}

/// A pattern containing rows of cells across channels.
#[derive(Clone, Debug)]
pub struct Pattern {
    /// Number of rows (typically 64, can be 1-256)
    pub rows: u16,
    /// Number of channels
    pub channels: u8,
    /// Ticks per row (default 6, affects timing resolution)
    pub ticks_per_row: u8,
    /// Pattern data, stored row-major: data[row * channels + channel]
    pub data: Vec<Cell>,
}

impl Pattern {
    /// Create a new pattern with empty cells.
    pub fn new(rows: u16, channels: u8) -> Self {
        Self {
            rows,
            channels,
            ticks_per_row: 6,
            data: alloc::vec![Cell::empty(); rows as usize * channels as usize],
        }
    }

    /// Get a reference to a cell.
    pub fn cell(&self, row: u16, channel: u8) -> &Cell {
        debug_assert!(row < self.rows);
        debug_assert!(channel < self.channels);
        &self.data[row as usize * self.channels as usize + channel as usize]
    }

    /// Get a mutable reference to a cell.
    pub fn cell_mut(&mut self, row: u16, channel: u8) -> &mut Cell {
        debug_assert!(row < self.rows);
        debug_assert!(channel < self.channels);
        &mut self.data[row as usize * self.channels as usize + channel as usize]
    }

    /// Iterate over all cells in a row.
    pub fn row(&self, row: u16) -> &[Cell] {
        let start = row as usize * self.channels as usize;
        &self.data[start..start + self.channels as usize]
    }

    /// Total number of ticks in this pattern.
    pub fn total_ticks(&self) -> u64 {
        self.rows as u64 * self.ticks_per_row as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_octave_semitone() {
        let c4 = Note::from_octave_semitone(4, 0);
        assert_eq!(c4, Note::On(48));
        assert_eq!(c4.octave(), Some(4));
        assert_eq!(c4.semitone(), Some(0));

        let a4 = Note::from_octave_semitone(4, 9);
        assert_eq!(a4, Note::On(57));
    }

    #[test]
    fn pattern_cell_access() {
        let mut pattern = Pattern::new(64, 4);
        pattern.cell_mut(10, 2).note = Note::On(60);

        assert_eq!(pattern.cell(10, 2).note, Note::On(60));
        assert_eq!(pattern.cell(10, 1).note, Note::None);
    }
}
