//! Song feature analysis — scans a Song to report which features are used.

use alloc::collections::BTreeSet;
use core::fmt;

use crate::pattern::Note;
use crate::song::Song;

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
