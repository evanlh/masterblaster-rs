//! Song feature analysis — scans cells, patterns, or whole songs to report which features are used.

use alloc::collections::BTreeSet;
use core::fmt;

use crate::pattern::{Cell, Note, Pattern};
use crate::song::{OrderEntry, Song};

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

/// Summary of features used across an entire song.
pub struct SongFeatures {
    pub pattern: PatternFeatures,
    pub samples_with_loops: usize,
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

// --- Aggregation ---

/// Merge `other` into `self`, combining all sets and ranges.
fn merge_into(base: &mut PatternFeatures, other: &PatternFeatures) {
    base.effects.extend(&other.effects);
    base.volume_commands.extend(&other.volume_commands);
    base.has_note_off |= other.has_note_off;
    base.has_note_fade |= other.has_note_fade;
    base.total_notes += other.total_notes;
    base.instruments_used.extend(&other.instruments_used);
    base.note_range = match (base.note_range, other.note_range) {
        (Some((a_lo, a_hi)), Some((b_lo, b_hi))) => Some((a_lo.min(b_lo), a_hi.max(b_hi))),
        (a, None) => a,
        (None, b) => b,
    };
}

// --- Song-level analysis ---

/// Analyze an entire song (all patterns).
pub fn analyze(song: &Song) -> SongFeatures {
    let mut combined = PatternFeatures::default();
    for pattern in &song.patterns {
        let pf = analyze_pattern(pattern);
        merge_into(&mut combined, &pf);
    }
    SongFeatures {
        pattern: combined,
        samples_with_loops: song.samples.iter().filter(|s| s.has_loop()).count(),
    }
}

// --- Display ---

fn fmt_features(feat: &PatternFeatures, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(f, "Notes:    {} total", feat.total_notes)?;
    if let Some((lo, hi)) = feat.note_range {
        writeln!(f, "Range:    {} - {} (MIDI)", lo, hi)?;
    }
    writeln!(
        f,
        "Note types: On{}{}",
        if feat.has_note_off { ", Off" } else { "" },
        if feat.has_note_fade { ", Fade" } else { "" },
    )?;
    writeln!(f, "Instruments: {} used", feat.instruments_used.len())?;

    if feat.effects.is_empty() {
        writeln!(f, "Effects:  (none)")?;
    } else {
        let effects: alloc::vec::Vec<&str> = feat.effects.iter().copied().collect();
        writeln!(f, "Effects:  {}", effects.join(", "))?;
    }

    if !feat.volume_commands.is_empty() {
        let cmds: alloc::vec::Vec<&str> = feat.volume_commands.iter().copied().collect();
        writeln!(f, "VolCmds:  {}", cmds.join(", "))?;
    }

    Ok(())
}

impl fmt::Display for PatternFeatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Pattern Features:")?;
        fmt_features(self, f)
    }
}

impl fmt::Display for SongFeatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Song Features:")?;
        fmt_features(&self.pattern, f)?;
        writeln!(f, "Loops:    {} samples with loops", self.samples_with_loops)
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

    // --- Song-level tests (aggregation) ---

    #[test]
    fn song_aggregates_across_patterns() {
        let mut song = Song::with_channels("test", 1);

        let mut p0 = Pattern::new(4, 1);
        p0.cell_mut(0, 0).note = Note::On(48);
        p0.cell_mut(0, 0).instrument = 1;
        p0.cell_mut(0, 0).effect = Effect::VolumeSlide(4);
        let idx0 = song.add_pattern(p0);

        let mut p1 = Pattern::new(4, 1);
        p1.cell_mut(0, 0).note = Note::On(72);
        p1.cell_mut(0, 0).instrument = 3;
        p1.cell_mut(0, 0).effect = Effect::Vibrato { speed: 4, depth: 2 };
        let idx1 = song.add_pattern(p1);

        song.add_order(OrderEntry::Pattern(idx0));
        song.add_order(OrderEntry::Pattern(idx1));

        let f = analyze(&song);
        assert_eq!(f.pattern.total_notes, 2);
        assert_eq!(f.pattern.note_range, Some((48, 72)));
        assert!(f.pattern.effects.contains("VolumeSlide"));
        assert!(f.pattern.effects.contains("Vibrato"));
        assert!(f.pattern.instruments_used.contains(&1));
        assert!(f.pattern.instruments_used.contains(&3));
    }

    #[test]
    fn empty_song_has_no_features() {
        let song = one_pattern_song(Pattern::new(4, 1));
        let f = analyze(&song);
        assert!(f.pattern.effects.is_empty());
        assert_eq!(f.pattern.total_notes, 0);
        assert_eq!(f.pattern.note_range, None);
    }

    // --- Playback position tests ---

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
        let pos = tick_to_position(&song, 12).unwrap();
        assert_eq!(pos.row, 2);
    }

    #[test]
    fn tick_to_position_second_order_entry() {
        let mut song = Song::with_channels("test", 1);
        let p0 = song.add_pattern(Pattern::new(4, 1));
        let p1 = song.add_pattern(Pattern::new(8, 1));
        song.add_order(OrderEntry::Pattern(p0));
        song.add_order(OrderEntry::Pattern(p1));

        let pos = tick_to_position(&song, 24).unwrap();
        assert_eq!(pos.order_index, 1);
        assert_eq!(pos.pattern_index, p1);
        assert_eq!(pos.row, 0);

        let pos = tick_to_position(&song, 30).unwrap();
        assert_eq!(pos.row, 1);
    }

    #[test]
    fn tick_to_position_past_end_returns_none() {
        let song = one_pattern_song(Pattern::new(4, 1));
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
}
