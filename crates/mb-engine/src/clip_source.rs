//! ClipSourceState — lazy, incremental event source for a single track.
//!
//! Walks a track's sequence entries and clips row by row, generating events
//! on demand as playback advances. Mirrors the logic of `schedule_track` but
//! reads pattern data lazily so edits ahead of the cursor take effect.

use alloc::vec::Vec;
use mb_ir::{Effect, Event, MusicalTime, Song, Track};

use crate::event_source::EventSource;
use crate::scheduler::{schedule_cell, target_for_track_column};

/// Incremental event source for one track.
#[derive(Clone, Debug)]
pub struct ClipSourceState {
    /// Index into `song.tracks`
    track_idx: usize,
    /// Current sequence entry index
    seq_idx: usize,
    /// Current row within the current clip
    row: u16,
    /// Current musical time
    time: MusicalTime,
    /// Current speed (ticks per row), updated by SetSpeed effects
    speed: u32,
    /// Song-level rows per beat
    song_rpb: u32,
    /// Max rows for loop detection
    max_rows: u64,
    /// Rows processed so far
    rows_processed: u64,
    /// Whether this source is exhausted
    exhausted: bool,
    /// The time at which this source became exhausted (accounts for PatternBreak/PositionJump).
    end_time: Option<MusicalTime>,
}

impl ClipSourceState {
    /// Create a new ClipSourceState for a given track.
    pub fn new(song: &Song, track_idx: usize) -> Self {
        let track = &song.tracks[track_idx];
        let time = track.sequence.first()
            .map(|e| e.start)
            .unwrap_or(MusicalTime::zero());
        let exhausted = track.sequence.is_empty()
            || track.muted
            || !song.is_tracker(track);
        Self {
            track_idx,
            seq_idx: 0,
            row: 0,
            time,
            speed: song.initial_speed as u32,
            song_rpb: song.rows_per_beat as u32,
            max_rows: compute_max_rows(track),
            rows_processed: 0,
            exhausted,
            end_time: if exhausted { Some(MusicalTime::zero()) } else { None },
        }
    }

    /// Update the internal speed (called when a SetSpeed event is observed).
    pub fn set_speed(&mut self, speed: u8) {
        self.speed = speed as u32;
    }

    /// The time at which this source became exhausted.
    pub fn end_time(&self) -> Option<MusicalTime> {
        self.end_time
    }
}

/// Compute max rows for loop detection (same as scheduler.rs).
fn compute_max_rows(track: &Track) -> u64 {
    let channels = (track.num_channels as u64).max(1);
    let from_clips: u64 = track.clips.iter()
        .filter_map(|c| c.pattern().map(|p| p.rows as u64))
        .sum();
    let from_seq: u64 = track.sequence.iter()
        .map(|e| e.length as u64)
        .sum();
    from_clips.max(from_seq) * channels * 2 + 256
}

/// Resolve effective speed for a pattern.
fn effective_speed(pattern: &mb_ir::Pattern, global_speed: u32) -> u32 {
    if pattern.ticks_per_row > 0 {
        pattern.ticks_per_row as u32
    } else {
        global_speed
    }
}

/// Flow control state from a pattern row.
struct FlowControl {
    break_row: Option<u8>,
    jump_order: Option<u8>,
    new_speed: Option<u32>,
    pattern_delay: u8,
}

/// Scan flow control effects across all columns of a row.
fn scan_row_flow_control(pattern: &mb_ir::Pattern, row: u16) -> FlowControl {
    let mut fc = FlowControl {
        break_row: None,
        jump_order: None,
        new_speed: None,
        pattern_delay: 0,
    };
    if row >= pattern.rows { return fc; }
    for col in 0..pattern.channels {
        match pattern.cell(row, col).effect {
            Effect::PatternBreak(r) => fc.break_row = Some(r),
            Effect::PositionJump(p) => fc.jump_order = Some(p),
            Effect::SetSpeed(s) if s > 0 => fc.new_speed = Some(s as u32),
            Effect::PatternDelay(d) => fc.pattern_delay = d,
            _ => {}
        }
    }
    fc
}

/// Get the start time for the next sequence entry.
fn advance_to_seq_entry(track: &Track, seq_idx: usize, current: MusicalTime) -> MusicalTime {
    track.sequence.get(seq_idx).map_or(current, |e| e.start)
}

impl EventSource for ClipSourceState {
    fn drain_until(&mut self, time: MusicalTime, song: &Song, out: &mut Vec<Event>) -> usize {
        let start_len = out.len();

        while !self.exhausted && self.time <= time {
            let track = &song.tracks[self.track_idx];

            if self.seq_idx >= track.sequence.len() {
                self.exhausted = true;
                self.end_time = Some(self.time);
                break;
            }

            let entry = &track.sequence[self.seq_idx];
            let entry_length = entry.length;
            let clip_idx = entry.clip_idx as usize;

            let clip = match track.get_pattern_at(clip_idx) {
                Some(p) => p,
                None => {
                    self.seq_idx += 1;
                    self.row = 0;
                    self.time = advance_to_seq_entry(track, self.seq_idx, self.time);
                    continue;
                }
            };

            let num_rows = entry_length.min(clip.rows);
            let rpb = clip.rows_per_beat.map_or(self.song_rpb, |r| r as u32);
            let eff_speed = effective_speed(clip, self.speed);

            // Check if we've passed the next entry's start
            let next_start = track.sequence.get(self.seq_idx + 1).map(|e| e.start);
            if let Some(ns) = next_start {
                if self.time >= ns {
                    self.seq_idx += 1;
                    self.row = 0;
                    self.time = ns;
                    continue;
                }
            }

            if self.row >= num_rows {
                self.seq_idx += 1;
                self.row = 0;
                self.time = advance_to_seq_entry(track, self.seq_idx, self.time);
                continue;
            }

            // Schedule all columns at this row
            for col in 0..clip.channels {
                let target = target_for_track_column(track, col);
                schedule_cell(clip.cell(self.row, col), self.time, target, eff_speed, rpb, out);
            }

            let fc = scan_row_flow_control(clip, self.row);
            if let Some(s) = fc.new_speed {
                self.speed = s;
            }

            self.time = self.time.add_rows(1 + fc.pattern_delay as u32, rpb);
            self.rows_processed += 1;
            if self.rows_processed >= self.max_rows {
                self.exhausted = true;
                self.end_time = Some(self.time);
                break;
            }

            match (fc.jump_order, fc.break_row) {
                (Some(pos), Some(r)) => { self.seq_idx = pos as usize; self.row = r as u16; }
                (Some(pos), None) => { self.seq_idx = pos as usize; self.row = 0; }
                (None, Some(r)) => { self.seq_idx += 1; self.row = r as u16; }
                (None, None) => {
                    self.row += 1;
                    if self.row >= num_rows {
                        self.seq_idx += 1;
                        self.row = 0;
                        self.time = advance_to_seq_entry(track, self.seq_idx, self.time);
                    }
                }
            }
        }

        out.len() - start_len
    }

    fn seek(&mut self, _time: MusicalTime, song: &Song) {
        *self = Self::new(song, self.track_idx);
    }

    fn peek_time(&self) -> Option<MusicalTime> {
        if self.exhausted { None } else { Some(self.time) }
    }
}

/// Compute the total end time from all clip sources.
pub fn sources_end_time(_sources: &[ClipSourceState], song: &Song) -> MusicalTime {
    song.total_time()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use mb_ir::{build_tracks, Note, OrderEntry, Pattern, VolumeCommand};
    use crate::scheduler;

    /// Build a minimal 1-channel song with a single pattern.
    fn one_channel_song(pattern: Pattern) -> Song {
        let mut song = Song::with_channels("test", 1);
        let patterns = vec![pattern];
        let order = vec![OrderEntry::Pattern(0)];
        build_tracks(&mut song, &patterns, &order);
        song
    }

    /// Build a song from patterns + order.
    fn song_from(channels: u8, patterns: Vec<Pattern>, order: Vec<OrderEntry>) -> Song {
        let mut song = Song::with_channels("test", channels);
        build_tracks(&mut song, &patterns, &order);
        song
    }

    /// Drain a ClipSource to completion, collecting all events.
    fn drain_all(song: &Song, track_idx: usize) -> Vec<Event> {
        let mut source = ClipSourceState::new(song, track_idx);
        let mut events = Vec::new();
        let far_future = MusicalTime::from_beats(10000);
        source.drain_until(far_future, song, &mut events);
        events
    }

    /// Drain all sources and compare with schedule_song output.
    fn assert_matches_schedule_song(song: &Song) {
        let expected = scheduler::schedule_song(song);
        let mut expected_events = expected.events;
        expected_events.sort_by(|a, b| a.time.cmp(&b.time));

        let mut actual_events = Vec::new();
        for track_idx in 0..song.tracks.len() {
            let mut source = ClipSourceState::new(song, track_idx);
            let far_future = MusicalTime::from_beats(10000);
            source.drain_until(far_future, song, &mut actual_events);
        }
        actual_events.sort_by(|a, b| a.time.cmp(&b.time));

        assert_eq!(
            actual_events.len(), expected_events.len(),
            "event count mismatch: clip_source={}, schedule_song={}",
            actual_events.len(), expected_events.len()
        );

        for (i, (actual, expected)) in actual_events.iter().zip(expected_events.iter()).enumerate() {
            assert_eq!(
                actual.time, expected.time,
                "time mismatch at event {}: {:?} vs {:?}",
                i, actual, expected
            );
            assert_eq!(
                actual.target, expected.target,
                "target mismatch at event {}: {:?} vs {:?}",
                i, actual, expected
            );
            assert_eq!(
                actual.payload, expected.payload,
                "payload mismatch at event {}: {:?} vs {:?}",
                i, actual, expected
            );
        }
    }

    #[test]
    fn empty_pattern_produces_no_events() {
        let song = one_channel_song(Pattern::new(4, 1));
        let events = drain_all(&song, 0);
        assert!(events.is_empty());
    }

    #[test]
    fn single_note_matches_scheduler() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        assert_matches_schedule_song(&one_channel_song(pat));
    }

    #[test]
    fn multiple_channels_matches_scheduler() {
        let mut pat = Pattern::new(4, 3);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 2).note = Note::On(64);
        pat.cell_mut(0, 2).instrument = 1;
        assert_matches_schedule_song(&song_from(3, vec![pat], vec![OrderEntry::Pattern(0)]));
    }

    #[test]
    fn two_patterns_matches_scheduler() {
        let mut pat0 = Pattern::new(4, 1);
        pat0.cell_mut(0, 0).note = Note::On(60);
        pat0.cell_mut(0, 0).instrument = 1;
        let mut pat1 = Pattern::new(4, 1);
        pat1.cell_mut(0, 0).note = Note::On(64);
        pat1.cell_mut(0, 0).instrument = 1;
        assert_matches_schedule_song(&song_from(
            1, vec![pat0, pat1],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1)],
        ));
    }

    #[test]
    fn repeated_pattern_matches_scheduler() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        assert_matches_schedule_song(&song_from(
            1, vec![pat],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(0)],
        ));
    }

    #[test]
    fn pattern_break_matches_scheduler() {
        let mut pat0 = Pattern::new(4, 1);
        pat0.cell_mut(0, 0).note = Note::On(60);
        pat0.cell_mut(0, 0).instrument = 1;
        pat0.cell_mut(1, 0).effect = Effect::PatternBreak(0);
        let mut pat1 = Pattern::new(4, 1);
        pat1.cell_mut(0, 0).note = Note::On(64);
        pat1.cell_mut(0, 0).instrument = 1;
        assert_matches_schedule_song(&song_from(
            1, vec![pat0, pat1],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1)],
        ));
    }

    #[test]
    fn position_jump_matches_scheduler() {
        let mut pat0 = Pattern::new(2, 1);
        pat0.cell_mut(0, 0).note = Note::On(60);
        pat0.cell_mut(0, 0).instrument = 1;
        pat0.cell_mut(0, 0).effect = Effect::PositionJump(2);
        let mut pat1 = Pattern::new(2, 1);
        pat1.cell_mut(0, 0).note = Note::On(62);
        pat1.cell_mut(0, 0).instrument = 1;
        let mut pat2 = Pattern::new(2, 1);
        pat2.cell_mut(0, 0).note = Note::On(64);
        pat2.cell_mut(0, 0).instrument = 1;
        assert_matches_schedule_song(&song_from(
            1, vec![pat0, pat1, pat2],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1), OrderEntry::Pattern(2)],
        ));
    }

    #[test]
    fn set_speed_matches_scheduler() {
        let mut pat = Pattern::new(4, 1);
        pat.ticks_per_row = 0;
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(1, 0).effect = Effect::SetSpeed(3);
        pat.cell_mut(2, 0).note = Note::On(64);
        pat.cell_mut(2, 0).instrument = 1;
        assert_matches_schedule_song(&one_channel_song(pat));
    }

    #[test]
    fn note_delay_matches_scheduler() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::NoteDelay(3);
        assert_matches_schedule_song(&one_channel_song(pat));
    }

    #[test]
    fn pattern_delay_matches_scheduler() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).effect = Effect::PatternDelay(1);
        pat.cell_mut(1, 0).note = Note::On(60);
        pat.cell_mut(1, 0).instrument = 1;
        assert_matches_schedule_song(&one_channel_song(pat));
    }

    #[test]
    fn effects_and_volume_match_scheduler() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::VolumeSlide(4);
        pat.cell_mut(0, 0).volume = VolumeCommand::Volume(48);
        assert_matches_schedule_song(&one_channel_song(pat));
    }

    #[test]
    fn incremental_drain_matches_full_drain() {
        let mut pat = Pattern::new(8, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(4, 0).note = Note::On(64);
        pat.cell_mut(4, 0).instrument = 1;
        let song = one_channel_song(pat);

        // Drain incrementally (one beat at a time)
        let mut source = ClipSourceState::new(&song, 0);
        let mut incremental = Vec::new();
        for beat in 0..10 {
            source.drain_until(MusicalTime::from_beats(beat), &song, &mut incremental);
        }

        // Drain all at once
        let full = drain_all(&song, 0);

        assert_eq!(incremental.len(), full.len());
        for (i, (a, b)) in incremental.iter().zip(full.iter()).enumerate() {
            assert_eq!(a.time, b.time, "time mismatch at {}", i);
            assert_eq!(a.payload, b.payload, "payload mismatch at {}", i);
        }
    }

    #[test]
    fn exhausted_source_returns_none_peek() {
        let song = one_channel_song(Pattern::new(4, 1));
        let mut source = ClipSourceState::new(&song, 0);
        source.drain_until(MusicalTime::from_beats(10000), &song, &mut Vec::new());
        assert!(source.peek_time().is_none());
    }

    #[test]
    fn tone_porta_matches_scheduler() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::TonePorta(8);
        assert_matches_schedule_song(&one_channel_song(pat));
    }
}
