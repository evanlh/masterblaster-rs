//! Song feature analysis — scans a Song to report which features are used.

use alloc::collections::BTreeSet;
use core::fmt;

use crate::pattern::Note;
use crate::song::{OrderEntry, Song};

/// Summary of features used in a song.
pub struct SongFeatures {
    pub effects: BTreeSet<&'static str>,
    pub volume_commands: BTreeSet<&'static str>,
    pub has_note_off: bool,
    pub has_note_fade: bool,
    pub note_range: Option<(u8, u8)>,
    pub instruments_used: BTreeSet<u8>,
    pub samples_with_loops: usize,
    pub total_notes: usize,
}

/// Analyze a song and return a summary of which features it uses.
pub fn analyze(song: &Song) -> SongFeatures {
    let mut features = SongFeatures {
        effects: BTreeSet::new(),
        volume_commands: BTreeSet::new(),
        has_note_off: false,
        has_note_fade: false,
        note_range: None,
        instruments_used: BTreeSet::new(),
        samples_with_loops: song.samples.iter().filter(|s| s.has_loop()).count(),
        total_notes: 0,
    };

    for pattern in &song.patterns {
        for cell in &pattern.data {
            analyze_cell(cell, &mut features);
        }
    }

    features
}

fn analyze_cell(cell: &crate::pattern::Cell, features: &mut SongFeatures) {
    match cell.note {
        Note::On(n) => {
            features.total_notes += 1;
            features.note_range = Some(match features.note_range {
                Some((lo, hi)) => (lo.min(n), hi.max(n)),
                None => (n, n),
            });
        }
        Note::Off => features.has_note_off = true,
        Note::Fade => features.has_note_fade = true,
        Note::None => {}
    }

    if cell.instrument > 0 {
        features.instruments_used.insert(cell.instrument);
    }

    let eff_name = cell.effect.name();
    if eff_name != "None" {
        features.effects.insert(eff_name);
    }

    let vol_name = cell.volume.name();
    if vol_name != "None" {
        features.volume_commands.insert(vol_name);
    }
}

impl fmt::Display for SongFeatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Notes:    {} total", self.total_notes)?;
        if let Some((lo, hi)) = self.note_range {
            writeln!(f, "Range:    {} - {} (MIDI)", lo, hi)?;
        }
        writeln!(
            f,
            "Note types: On{}{}",
            if self.has_note_off { ", Off" } else { "" },
            if self.has_note_fade { ", Fade" } else { "" },
        )?;
        writeln!(
            f,
            "Instruments: {} used, {} samples with loops",
            self.instruments_used.len(),
            self.samples_with_loops,
        )?;

        if self.effects.is_empty() {
            writeln!(f, "Effects:  (none)")?;
        } else {
            let effects: alloc::vec::Vec<&str> = self.effects.iter().copied().collect();
            writeln!(f, "Effects:  {}", effects.join(", "))?;
        }

        if !self.volume_commands.is_empty() {
            let cmds: alloc::vec::Vec<&str> = self.volume_commands.iter().copied().collect();
            writeln!(f, "VolCmds:  {}", cmds.join(", "))?;
        }

        Ok(())
    }
}

// --- Playback position ---

/// A position within the song's order/pattern structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackPosition {
    pub order_index: usize,
    pub pattern_index: u8,
    pub row: u16,
}

/// Map an absolute tick to a position in the song (order index, pattern, row).
///
/// Uses the same tick accumulation logic as the scheduler.
/// Returns `None` if the tick is past the song's end.
pub fn tick_to_position(song: &Song, tick: u64) -> Option<PlaybackPosition> {
    let speed = song.initial_speed as u64;
    let mut accumulated: u64 = 0;

    for (order_index, entry) in song.order.iter().enumerate() {
        match entry {
            OrderEntry::Pattern(idx) => {
                let pattern = song.patterns.get(*idx as usize)?;
                let tpr = pattern.ticks_per_row as u64;
                let row_speed = if tpr > 0 { tpr } else { speed };
                let pattern_ticks = pattern.rows as u64 * row_speed;

                if tick < accumulated + pattern_ticks {
                    let offset = tick - accumulated;
                    let row = (offset / row_speed) as u16;
                    return Some(PlaybackPosition {
                        order_index,
                        pattern_index: *idx,
                        row,
                    });
                }
                accumulated += pattern_ticks;
            }
            OrderEntry::Skip => {}
            OrderEntry::End => break,
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{Effect, VolumeCommand};
    use crate::pattern::{Note, Pattern};
    use crate::song::OrderEntry;

    fn one_pattern_song(pat: Pattern) -> Song {
        let mut song = Song::with_channels("test", 1);
        let idx = song.add_pattern(pat);
        song.add_order(OrderEntry::Pattern(idx));
        song
    }

    #[test]
    fn empty_song_has_no_features() {
        let song = one_pattern_song(Pattern::new(4, 1));
        let f = analyze(&song);
        assert!(f.effects.is_empty());
        assert!(f.volume_commands.is_empty());
        assert_eq!(f.total_notes, 0);
        assert_eq!(f.note_range, None);
        assert!(!f.has_note_off);
        assert!(!f.has_note_fade);
    }

    #[test]
    fn detects_notes_and_instruments() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(48);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(1, 0).note = Note::On(60);
        pat.cell_mut(1, 0).instrument = 2;
        pat.cell_mut(2, 0).note = Note::Off;

        let f = analyze(&one_pattern_song(pat));
        assert_eq!(f.total_notes, 2);
        assert_eq!(f.note_range, Some((48, 60)));
        assert!(f.has_note_off);
        assert!(!f.has_note_fade);
        assert_eq!(f.instruments_used.len(), 2);
        assert!(f.instruments_used.contains(&1));
        assert!(f.instruments_used.contains(&2));
    }

    #[test]
    fn tick_to_position_first_row() {
        let song = one_pattern_song(Pattern::new(4, 1));
        let pos = tick_to_position(&song, 0).unwrap();
        assert_eq!(pos.order_index, 0);
        assert_eq!(pos.pattern_index, 0);
        assert_eq!(pos.row, 0);
    }

    #[test]
    fn tick_to_position_mid_pattern() {
        let song = one_pattern_song(Pattern::new(8, 1)); // ticks_per_row=6
        // tick 12 = row 2 (12 / 6)
        let pos = tick_to_position(&song, 12).unwrap();
        assert_eq!(pos.row, 2);
    }

    #[test]
    fn tick_to_position_second_order_entry() {
        let mut song = Song::with_channels("test", 1);
        let p0 = song.add_pattern(Pattern::new(4, 1)); // 4*6=24 ticks
        let p1 = song.add_pattern(Pattern::new(8, 1)); // 8*6=48 ticks
        song.add_order(OrderEntry::Pattern(p0));
        song.add_order(OrderEntry::Pattern(p1));

        // tick 24 = first row of second pattern
        let pos = tick_to_position(&song, 24).unwrap();
        assert_eq!(pos.order_index, 1);
        assert_eq!(pos.pattern_index, p1);
        assert_eq!(pos.row, 0);

        // tick 30 = row 1 of second pattern
        let pos = tick_to_position(&song, 30).unwrap();
        assert_eq!(pos.row, 1);
    }

    #[test]
    fn tick_to_position_past_end_returns_none() {
        let song = one_pattern_song(Pattern::new(4, 1)); // 24 ticks total
        assert!(tick_to_position(&song, 24).is_none());
        assert!(tick_to_position(&song, 100).is_none());
    }

    #[test]
    fn tick_to_position_skips_order_skip() {
        let mut song = Song::with_channels("test", 1);
        let p0 = song.add_pattern(Pattern::new(4, 1));
        song.add_order(OrderEntry::Skip);
        song.add_order(OrderEntry::Pattern(p0));

        let pos = tick_to_position(&song, 0).unwrap();
        assert_eq!(pos.order_index, 1);
        assert_eq!(pos.pattern_index, p0);
    }

    #[test]
    fn detects_effects_and_volume_commands() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).effect = Effect::VolumeSlide(4);
        pat.cell_mut(1, 0).effect = Effect::SetSpeed(6);
        pat.cell_mut(2, 0).volume = VolumeCommand::Volume(48);

        let f = analyze(&one_pattern_song(pat));
        assert!(f.effects.contains("VolumeSlide"));
        assert!(f.effects.contains("SetSpeed"));
        assert_eq!(f.effects.len(), 2);
        assert!(f.volume_commands.contains("Volume"));
        assert_eq!(f.volume_commands.len(), 1);
    }
}
