//! Song feature analysis — scans cells, patterns, or whole songs to report which features are used.

use alloc::collections::BTreeSet;
use core::fmt;

use crate::musical_time::MusicalTime;
use crate::pattern::{Cell, Note, Pattern};
use crate::song::Song;

/// Features found in a single pattern (or any collection of cells).
#[derive(Clone, Debug, Default)]
pub struct PatternFeatures {
    pub effects: BTreeSet<&'static str>,
    pub volume_commands: BTreeSet<&'static str>,
    pub has_note_off: bool,
    pub has_note_fade: bool,
    pub note_range: Option<(u8, u8)>,
    pub instruments_used: BTreeSet<u8>,
    pub total_notes: usize,
}

// --- Cell-level analysis ---

fn accumulate_cell(cell: &Cell, feat: &mut PatternFeatures) {
    match cell.note {
        Note::On(n) => {
            feat.total_notes += 1;
            feat.note_range = Some(match feat.note_range {
                Some((lo, hi)) => (lo.min(n), hi.max(n)),
                None => (n, n),
            });
        }
        Note::Off => feat.has_note_off = true,
        Note::Fade => feat.has_note_fade = true,
        Note::None => {}
    }

    if cell.instrument > 0 {
        feat.instruments_used.insert(cell.instrument);
    }

    let eff_name = cell.effect.name();
    if eff_name != "None" {
        feat.effects.insert(eff_name);
    }

    let vol_name = cell.volume.name();
    if vol_name != "None" {
        feat.volume_commands.insert(vol_name);
    }
}

// --- Pattern-level analysis ---

/// Analyze a single pattern.
pub fn analyze_pattern(pattern: &Pattern) -> PatternFeatures {
    let mut feat = PatternFeatures::default();
    for cell in &pattern.data {
        accumulate_cell(cell, &mut feat);
    }
    feat
}

// --- Display ---

impl fmt::Display for PatternFeatures {
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
        writeln!(f, "Instruments: {} used", self.instruments_used.len())?;

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

/// A position within the song's order/pattern structure (legacy, kept for compatibility).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackPosition {
    pub order_index: usize,
    pub pattern_index: u8,
    pub row: u16,
}

/// A position within the per-track sequencing model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackPlaybackPosition {
    pub group: Option<u16>,
    pub seq_index: usize,
    pub clip_idx: u16,
    pub row: u16,
}

/// Map a `MusicalTime` to a position within the per-track sequencing model.
///
/// Uses the first track in the given group to walk the sequence.
/// Returns `None` if time is past the end or no matching group.
pub fn time_to_track_position(song: &Song, time: MusicalTime, group: Option<u16>) -> Option<TrackPlaybackPosition> {
    let track = song.tracks.iter().find(|t| t.group == group)?;
    let rpb = song.rows_per_beat as u32;

    for (seq_index, entry) in track.sequence.iter().enumerate() {
        let clip = track.clips.get(entry.clip_idx as usize)?;
        let pattern = clip.pattern()?;
        let pat_rpb = pattern.rows_per_beat.map_or(rpb, |r| r as u32);
        let clip_end = entry.start.add_rows(pattern.rows as u32, pat_rpb);

        if time < clip_end {
            let row = find_row_at(entry.start, time, pat_rpb, pattern.rows);
            return Some(TrackPlaybackPosition {
                group,
                seq_index,
                clip_idx: entry.clip_idx,
                row,
            });
        }
    }

    None
}

/// Find which row contains `time`, given that the pattern starts at `base`.
fn find_row_at(base: MusicalTime, time: MusicalTime, rpb: u32, max_rows: u16) -> u16 {
    if time < base || rpb == 0 || max_rows == 0 {
        return 0;
    }
    let sub_per_row = crate::musical_time::SUB_BEAT_UNIT / rpb;
    let elapsed_beats = time.beat - base.beat;
    let elapsed_sub = elapsed_beats * crate::musical_time::SUB_BEAT_UNIT as u64
        + time.sub_beat as u64
        - base.sub_beat as u64;
    let row = (elapsed_sub / sub_per_row as u64) as u16;
    row.min(max_rows - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{Effect, VolumeCommand};
    use crate::pattern::{Note, Pattern};
    use crate::song::{build_tracks, OrderEntry};

    fn one_track_song(rows: u16) -> Song {
        let mut song = Song::with_channels("test", 1);
        let patterns = vec![Pattern::new(rows, 1)];
        let order = vec![OrderEntry::Pattern(0)];
        build_tracks(&mut song, &patterns, &order);
        song
    }

    // --- Pattern-level tests ---

    #[test]
    fn empty_pattern_has_no_features() {
        let pat = Pattern::new(4, 1);
        let f = analyze_pattern(&pat);
        assert!(f.effects.is_empty());
        assert!(f.volume_commands.is_empty());
        assert_eq!(f.total_notes, 0);
        assert_eq!(f.note_range, None);
        assert!(!f.has_note_off);
        assert!(!f.has_note_fade);
    }

    #[test]
    fn pattern_detects_notes_and_instruments() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(48);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(1, 0).note = Note::On(60);
        pat.cell_mut(1, 0).instrument = 2;
        pat.cell_mut(2, 0).note = Note::Off;

        let f = analyze_pattern(&pat);
        assert_eq!(f.total_notes, 2);
        assert_eq!(f.note_range, Some((48, 60)));
        assert!(f.has_note_off);
        assert!(!f.has_note_fade);
        assert_eq!(f.instruments_used.len(), 2);
    }

    #[test]
    fn pattern_detects_effects_and_volume_commands() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).effect = Effect::VolumeSlide(4);
        pat.cell_mut(1, 0).effect = Effect::SetSpeed(6);
        pat.cell_mut(2, 0).volume = VolumeCommand::Volume(48);

        let f = analyze_pattern(&pat);
        assert!(f.effects.contains("VolumeSlide"));
        assert!(f.effects.contains("SetSpeed"));
        assert_eq!(f.effects.len(), 2);
        assert!(f.volume_commands.contains("Volume"));
    }

    // --- Track playback position tests ---

    use crate::musical_time::{MusicalTime, SUB_BEAT_UNIT};

    fn time_at_row(row: u32) -> MusicalTime {
        MusicalTime::zero().add_rows(row, 4)
    }

    #[test]
    fn track_position_first_row() {
        let song = one_track_song(4);
        let pos = time_to_track_position(&song, MusicalTime::zero(), Some(0)).unwrap();
        assert_eq!(pos.seq_index, 0);
        assert_eq!(pos.clip_idx, 0);
        assert_eq!(pos.row, 0);
        assert_eq!(pos.group, Some(0));
    }

    #[test]
    fn track_position_mid_clip() {
        let song = one_track_song(8);
        let pos = time_to_track_position(&song, time_at_row(2), Some(0)).unwrap();
        assert_eq!(pos.row, 2);
    }

    #[test]
    fn track_position_second_seq_entry() {
        let mut song = Song::with_channels("test", 1);
        let patterns = vec![Pattern::new(4, 1), Pattern::new(8, 1)];
        let order = vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1)];
        build_tracks(&mut song, &patterns, &order);

        let pos = time_to_track_position(&song, MusicalTime::from_beats(1), Some(0)).unwrap();
        assert_eq!(pos.seq_index, 1);
        assert_eq!(pos.clip_idx, 1);
        assert_eq!(pos.row, 0);
    }

    #[test]
    fn track_position_past_end_returns_none() {
        let song = one_track_song(4);
        assert!(time_to_track_position(&song, MusicalTime::from_beats(1), Some(0)).is_none());
    }

    #[test]
    fn track_position_no_matching_group_returns_none() {
        let song = one_track_song(4);
        assert!(time_to_track_position(&song, MusicalTime::zero(), Some(99)).is_none());
    }

    #[test]
    fn track_position_sub_beat_within_row() {
        let song = one_track_song(8);
        let t = MusicalTime { beat: 0, sub_beat: SUB_BEAT_UNIT / 8 - 1 };
        let pos = time_to_track_position(&song, t, Some(0)).unwrap();
        assert_eq!(pos.row, 0);
    }
}
