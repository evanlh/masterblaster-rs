//! Pattern-to-event scheduling.
//!
//! Walks a song's per-track clips and sequences, producing a sorted Vec<Event>
//! that the engine can consume for playback.

use alloc::vec::Vec;
use mb_ir::{
    Cell, Effect, Event, EventPayload, EventTarget, MusicalTime, Note, Song,
    Track, VolumeCommand
};

/// Result of scheduling a song: events and total length.
pub struct ScheduleResult {
    pub events: Vec<Event>,
    pub total_time: MusicalTime,
}

/// Flow control state extracted from a pattern row.
struct FlowControl {
    break_row: Option<u8>,
    jump_order: Option<u8>,
    new_speed: Option<u32>,
    pattern_delay: u8,
}

/// Schedule all events from per-track clips + sequences.
pub fn schedule_song(song: &Song) -> ScheduleResult {
    let mut events = Vec::new();
    let mut max_time = MusicalTime::zero();

    for track in &song.tracks {
        if track.muted || !song.is_tracker(track) {
            continue;
        }
        let t = schedule_track(track, song, &mut events);
        if t > max_time { max_time = t; }
    }

    ScheduleResult { events, total_time: max_time }
}

/// Resolve effective speed for a pattern row.
fn effective_speed(pattern: &mb_ir::Pattern, global_speed: u32) -> u32 {
    if pattern.ticks_per_row > 0 {
        pattern.ticks_per_row as u32
    } else {
        global_speed
    }
}

/// Returns true if the effect is a tone portamento variant.
fn is_tone_porta(effect: &Effect) -> bool {
    matches!(effect, Effect::TonePorta(_) | Effect::TonePortaVolSlide(_))
}

/// Extract NoteDelay tick count from a cell's effect.
fn note_delay_amount(effect: &Effect) -> u32 {
    match effect {
        Effect::NoteDelay(d) if *d > 0 => *d as u32,
        _ => 0,
    }
}

/// Build the event target for a track column.
pub fn target_for_track_column(track: &Track, column: u8) -> EventTarget {
    match track.machine_node {
        Some(node_id) => EventTarget::NodeChannel(node_id, column),
        None => EventTarget::Channel(track.base_channel + column),
    }
}

/// Convert a single cell into events and append them to the output.
///
/// `speed` and `rpb` are needed for NoteDelay sub-beat computation:
/// ticks_per_beat = speed * rpb.
pub fn schedule_cell(
    cell: &Cell,
    time: MusicalTime,
    target: EventTarget,
    speed: u32,
    rpb: u32,
    events: &mut Vec<Event>,
) {
    let delay = note_delay_amount(&cell.effect);
    let tpb = speed * rpb;
    let note_time = time.add_ticks(delay, tpb);

    match cell.note {
        Note::On(note) => {
            if is_tone_porta(&cell.effect) {
                events.push(Event::new(
                    note_time,
                    target,
                    EventPayload::PortaTarget {
                        note,
                        instrument: cell.instrument,
                    },
                ));
            } else {
                events.push(Event::new(
                    note_time,
                    target,
                    EventPayload::NoteOn {
                        note,
                        velocity: 64,
                        instrument: cell.instrument,
                    },
                ));
            }
        }
        Note::Off | Note::Fade => {
            events.push(Event::new(
                note_time,
                target,
                EventPayload::NoteOff { note: 0 },
            ));
        }
        Note::None => {}
    }

    // Volume command is delayed with the note
    schedule_volume_command(&cell.volume, note_time, target, events);
    // Effect fires at row time (except NoteDelay/PatternDelay are consumed)
    schedule_effect(&cell.effect, time, target, events);
}

/// Convert a volume column command into an event.
fn schedule_volume_command(
    vol: &VolumeCommand,
    time: MusicalTime,
    target: EventTarget,
    events: &mut Vec<Event>,
) {
    let effect = match vol {
        VolumeCommand::None => return,
        VolumeCommand::Volume(v) => Effect::SetVolume(*v),
        VolumeCommand::Panning(p) => Effect::SetPan(*p),
        VolumeCommand::TonePorta(v) => Effect::TonePorta(*v),
        VolumeCommand::Vibrato(v) => Effect::Vibrato { speed: 0, depth: *v },
        VolumeCommand::VolumeSlideDown(v) => Effect::VolumeSlide(-(*v as i8)),
        VolumeCommand::VolumeSlideUp(v) => Effect::VolumeSlide(*v as i8),
        VolumeCommand::FineVolSlideDown(v) => Effect::FineVolumeSlideDown(*v),
        VolumeCommand::FineVolSlideUp(v) => Effect::FineVolumeSlideUp(*v),
        VolumeCommand::PortaDown(v) => Effect::PortaDown(*v),
        VolumeCommand::PortaUp(v) => Effect::PortaUp(*v),
    };
    events.push(Event::new(time, target, EventPayload::Effect(effect)));
}

/// Returns true if the effect is consumed by the scheduler (not emitted as event).
fn is_scheduler_directive(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::PatternBreak(_)
            | Effect::PositionJump(_)
            | Effect::PatternDelay(_)
            | Effect::NoteDelay(_)
    )
}

/// Convert an effect command into an event, routing tempo/speed to Global.
fn schedule_effect(effect: &Effect, time: MusicalTime, target: EventTarget, events: &mut Vec<Event>) {
    match effect {
        Effect::None => {}
        e if is_scheduler_directive(e) => {} // consumed by scheduler
        Effect::SetTempo(t) => {
            events.push(Event::new(
                time,
                EventTarget::Global,
                EventPayload::SetTempo(*t as u16 * 100),
            ));
        }
        Effect::SetSpeed(s) => {
            events.push(Event::new(
                time,
                EventTarget::Global,
                EventPayload::SetSpeed(*s),
            ));
        }
        other => {
            events.push(Event::new(
                time,
                target,
                EventPayload::Effect(*other),
            ));
        }
    }
}

/// Resolve engine channel index from a track column.
pub fn track_column_to_channel(track: &Track, column: u8) -> u8 {
    track.base_channel + column
}

/// Schedule events for a single track (walks sequence, iterates multi-channel patterns).
fn schedule_track(
    track: &Track,
    song: &Song,
    events: &mut Vec<Event>,
) -> MusicalTime {
    if track.sequence.is_empty() {
        return MusicalTime::zero();
    }

    let song_rpb = song.rows_per_beat as u32;
    let mut speed: u32 = song.initial_speed as u32;
    let mut seq_idx: usize = 0;
    let mut row: u16 = 0;
    let mut time = track.sequence[seq_idx].start;

    let max_rows = compute_max_rows(track);
    let mut rows_processed: u64 = 0;

    loop {
        if seq_idx >= track.sequence.len() { break; }
        let entry_length = track.sequence[seq_idx].length;

        let clip_idx = track.sequence[seq_idx].clip_idx as usize;
        let clip = match track.get_pattern_at(clip_idx) {
            Some(p) => p,
            None => { seq_idx += 1; row = 0; time = advance_to_seq_entry(track, seq_idx, time); continue; }
        };
        let num_rows = entry_length.min(clip.rows);
        let rpb = clip.rows_per_beat.map_or(song_rpb, |r| r as u32);
        let eff_speed = effective_speed(clip, speed);

        // Truncate: if current time has reached the next entry's start, advance
        let next_start = track.sequence.get(seq_idx + 1).map(|e| e.start);
        if let Some(ns) = next_start {
            if time >= ns {
                seq_idx += 1;
                row = 0;
                time = ns;
                continue;
            }
        }

        if row >= num_rows {
            seq_idx += 1;
            row = 0;
            time = advance_to_seq_entry(track, seq_idx, time);
            continue;
        }

        // Schedule all columns at this row
        for col in 0..clip.channels {
            let target = target_for_track_column(track, col);
            schedule_cell(clip.cell(row, col), time, target, eff_speed, rpb, events);
        }

        let fc = scan_row_flow_control(clip, row);
        if let Some(s) = fc.new_speed { speed = s; }

        time = time.add_rows(1 + fc.pattern_delay as u32, rpb);
        rows_processed += 1;
        if rows_processed >= max_rows { break; }

        match (fc.jump_order, fc.break_row) {
            // Flow control: keep linear time (SeqEntry.start assumes no breaks)
            (Some(pos), Some(r)) => { seq_idx = pos as usize; row = r as u16; }
            (Some(pos), None) => { seq_idx = pos as usize; row = 0; }
            (None, Some(r)) => { seq_idx += 1; row = r as u16; }
            // Normal advancement: use absolute SeqEntry.start
            (None, None) => {
                row += 1;
                if row >= num_rows {
                    seq_idx += 1;
                    row = 0;
                    time = advance_to_seq_entry(track, seq_idx, time);
                }
            }
        }
    }

    time
}


/// Get the start time for the next sequence entry, falling back to current time.
///
/// Used when seq_idx advances: sets time to the entry's absolute start position.
/// For MOD files (contiguous entries), this equals where the linear time would be.
/// For BMX files (absolute positions), this jumps to the correct timeline position.
fn advance_to_seq_entry(track: &Track, seq_idx: usize, current: MusicalTime) -> MusicalTime {
    track.sequence.get(seq_idx).map_or(current, |e| e.start)
}

/// Compute max rows for loop detection across all clips in a track.
///
/// Scales by num_channels to match pre-coalescing behavior where each channel
/// was a separate track contributing to the group's row budget.
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

/// Scan flow control effects across all columns of a pattern at a given row.
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

/// Find all MusicalTimes at which a given track clip + row appears in the sequence.
pub fn time_for_track_clip_row(
    track: &Track,
    clip_idx: u16,
    row: u16,
    song_rpb: u8,
) -> Vec<MusicalTime> {
    let rpb = song_rpb as u32;
    track.sequence.iter()
        .filter(|e| e.clip_idx == clip_idx && e.length > 0)
        .filter_map(|e| {
            let clip = track.clips.get(e.clip_idx as usize)?.pattern()?;
            let effective_rows = e.length.min(clip.rows);
            if row >= effective_rows { return None; }
            let pat_rpb = clip.rows_per_beat.map_or(rpb, |r| r as u32);
            Some(e.start.add_rows(row as u32, pat_rpb))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::{build_tracks, OrderEntry, Pattern};

    /// Schedule and return just the events (convenience for tests).
    fn schedule_events(song: &Song) -> Vec<Event> {
        schedule_song(song).events
    }

    /// Build a minimal 1-channel song with a single pattern via build_tracks.
    fn one_channel_song(pattern: Pattern) -> Song {
        let mut song = Song::with_channels("test", 1);
        let patterns = vec![pattern];
        let order = vec![OrderEntry::Pattern(0)];
        build_tracks(&mut song, &patterns, &order);
        song
    }

    /// Build a song from patterns + order via build_tracks.
    fn song_from(channels: u8, patterns: Vec<Pattern>, order: Vec<OrderEntry>) -> Song {
        let mut song = Song::with_channels("test", channels);
        build_tracks(&mut song, &patterns, &order);
        song
    }

    /// MusicalTime for row N at rpb=4 (default).
    fn time_at_row(n: u32) -> MusicalTime {
        MusicalTime::zero().add_rows(n, 4)
    }

    #[test]
    fn empty_pattern_produces_no_events() {
        let song = one_channel_song(Pattern::new(4, 1));
        let events = schedule_events(&song);
        assert!(events.is_empty());
    }

    #[test]
    fn single_note_at_row_zero() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].time, MusicalTime::zero());
        assert_eq!(events[0].target, EventTarget::NodeChannel(2, 0));
        assert_eq!(
            events[0].payload,
            EventPayload::NoteOn { note: 60, velocity: 64, instrument: 1 }
        );
    }

    #[test]
    fn note_at_row_n_offset_by_rows_per_beat() {
        let mut pat = Pattern::new(8, 1);
        pat.cell_mut(3, 0).note = Note::On(48);
        pat.cell_mut(3, 0).instrument = 2;

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].time, time_at_row(3));
    }

    #[test]
    fn note_off_produces_note_off_event() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(1, 0).note = Note::Off;

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].time, time_at_row(1));
        assert_eq!(events[0].payload, EventPayload::NoteOff { note: 0 });
    }

    #[test]
    fn multiple_channels_same_row() {
        let mut pat = Pattern::new(4, 3);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 2).note = Note::On(64);
        pat.cell_mut(0, 2).instrument = 1;

        let song = song_from(3, vec![pat], vec![OrderEntry::Pattern(0)]);
        let events = schedule_events(&song);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].target, EventTarget::NodeChannel(2, 0));
        assert_eq!(events[1].target, EventTarget::NodeChannel(2, 2));
        assert_eq!(events[0].time, MusicalTime::zero());
        assert_eq!(events[1].time, MusicalTime::zero());
    }

    #[test]
    fn two_patterns_in_order_offsets_correctly() {
        let mut pat0 = Pattern::new(4, 1);
        pat0.cell_mut(0, 0).note = Note::On(60);
        pat0.cell_mut(0, 0).instrument = 1;

        let mut pat1 = Pattern::new(4, 1);
        pat1.cell_mut(0, 0).note = Note::On(64);
        pat1.cell_mut(0, 0).instrument = 1;

        let song = song_from(1, vec![pat0, pat1],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1)]);
        let events = schedule_events(&song);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time, MusicalTime::zero());
        assert_eq!(events[1].time, MusicalTime::from_beats(1));
    }

    #[test]
    fn repeated_pattern_in_order() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;

        let song = song_from(1, vec![pat],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(0)]);
        let events = schedule_events(&song);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time, MusicalTime::zero());
        assert_eq!(events[1].time, MusicalTime::from_beats(1));
    }

    #[test]
    fn set_tempo_routes_to_global() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(1, 0).effect = Effect::SetTempo(140);

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].target, EventTarget::Global);
        assert_eq!(events[0].payload, EventPayload::SetTempo(14000));
    }

    #[test]
    fn set_speed_routes_to_global() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).effect = Effect::SetSpeed(3);

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].target, EventTarget::Global);
        assert_eq!(events[0].payload, EventPayload::SetSpeed(3));
    }

    #[test]
    fn effect_routes_to_channel() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).effect = Effect::VolumeSlide(4);

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].target, EventTarget::NodeChannel(2, 0));
        assert_eq!(events[0].payload, EventPayload::Effect(Effect::VolumeSlide(4)));
    }

    #[test]
    fn note_and_effect_on_same_cell() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::VolumeSlide(2);

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].payload, EventPayload::NoteOn { .. }));
        assert!(matches!(events[1].payload, EventPayload::Effect(_)));
        assert_eq!(events[0].time, events[1].time);
    }

    #[test]
    fn volume_column_produces_event() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).volume = VolumeCommand::Volume(48);

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload, EventPayload::Effect(Effect::SetVolume(48)));
    }

    #[test]
    fn order_end_stops_scheduling() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;

        let song = song_from(1, vec![pat],
            vec![OrderEntry::Pattern(0), OrderEntry::End, OrderEntry::Pattern(0)]);
        let events = schedule_events(&song);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn order_skip_is_ignored() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;

        let song = song_from(1, vec![pat],
            vec![OrderEntry::Pattern(0), OrderEntry::Skip, OrderEntry::Pattern(0)]);
        let events = schedule_events(&song);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time, MusicalTime::zero());
        assert_eq!(events[1].time, MusicalTime::from_beats(1));
    }

    #[test]
    fn total_time_matches_pattern_rows() {
        let pat = Pattern::new(4, 1);
        let result = schedule_song(&one_channel_song(pat));
        assert_eq!(result.total_time, MusicalTime::from_beats(1));
    }

    #[test]
    fn total_time_sums_across_order() {
        let song = song_from(1, vec![Pattern::new(8, 1)],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(0)]);
        let result = schedule_song(&song);
        assert_eq!(result.total_time, MusicalTime::from_beats(4));
    }

    #[test]
    fn tone_porta_with_note_emits_porta_target() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::TonePorta(8);

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].payload,
            EventPayload::PortaTarget { note: 60, instrument: 1 }
        );
        assert!(matches!(events[1].payload, EventPayload::Effect(Effect::TonePorta(8))));
    }

    #[test]
    fn tone_porta_vol_slide_with_note_emits_porta_target() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(64);
        pat.cell_mut(0, 0).instrument = 2;
        pat.cell_mut(0, 0).effect = Effect::TonePortaVolSlide(4);

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].payload,
            EventPayload::PortaTarget { note: 64, instrument: 2 }
        );
    }

    #[test]
    fn note_without_tone_porta_still_emits_note_on() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::VolumeSlide(4);

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].payload, EventPayload::NoteOn { .. }));
    }

    // --- PatternBreak tests ---

    #[test]
    fn pattern_break_skips_to_next_pattern() {
        let mut pat0 = Pattern::new(4, 1);
        pat0.cell_mut(0, 0).note = Note::On(60);
        pat0.cell_mut(0, 0).instrument = 1;
        pat0.cell_mut(1, 0).effect = Effect::PatternBreak(0);

        let mut pat1 = Pattern::new(4, 1);
        pat1.cell_mut(0, 0).note = Note::On(64);
        pat1.cell_mut(0, 0).instrument = 1;

        let result = schedule_song(&song_from(1, vec![pat0, pat1],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1)]));

        let notes: Vec<_> = result.events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some((note, e.time)),
            _ => None,
        }).collect();

        assert_eq!(notes[0], (60, MusicalTime::zero()));
        assert_eq!(notes[1], (64, time_at_row(2)));
    }

    #[test]
    fn pattern_break_to_specific_row() {
        let mut pat0 = Pattern::new(2, 1);
        pat0.cell_mut(0, 0).effect = Effect::PatternBreak(2);

        let mut pat1 = Pattern::new(4, 1);
        pat1.cell_mut(2, 0).note = Note::On(60);
        pat1.cell_mut(2, 0).instrument = 1;

        let events = schedule_events(&song_from(1, vec![pat0, pat1],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1)]));

        let notes: Vec<_> = events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some((note, e.time)),
            _ => None,
        }).collect();

        assert_eq!(notes, vec![(60, time_at_row(1))]);
    }

    #[test]
    fn pattern_break_total_time() {
        let mut pat0 = Pattern::new(4, 1);
        pat0.cell_mut(1, 0).effect = Effect::PatternBreak(0);

        let result = schedule_song(&song_from(1, vec![pat0, Pattern::new(4, 1)],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1)]));
        assert_eq!(result.total_time, time_at_row(6));
    }

    // --- PositionJump tests ---

    #[test]
    fn position_jump_to_later_order() {
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

        let events = schedule_events(&song_from(1, vec![pat0, pat1, pat2],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1), OrderEntry::Pattern(2)]));

        let notes: Vec<_> = events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some(note),
            _ => None,
        }).collect();
        assert_eq!(notes, vec![60, 64]);
        assert_eq!(events[0].time, MusicalTime::zero());
    }

    #[test]
    fn position_jump_backwards_terminates() {
        let mut pat = Pattern::new(2, 1);
        pat.cell_mut(1, 0).effect = Effect::PositionJump(0);

        let result = schedule_song(&one_channel_song(pat));
        assert!(result.total_time > MusicalTime::zero());
    }

    // --- Combined PatternBreak + PositionJump ---

    #[test]
    fn position_jump_with_pattern_break() {
        let mut pat0 = Pattern::new(2, 2);
        pat0.cell_mut(0, 0).effect = Effect::PositionJump(2);
        pat0.cell_mut(0, 1).effect = Effect::PatternBreak(1);

        let mut pat1 = Pattern::new(4, 2);
        pat1.cell_mut(0, 0).note = Note::On(62);
        pat1.cell_mut(0, 0).instrument = 1;

        let mut pat2 = Pattern::new(4, 2);
        pat2.cell_mut(1, 0).note = Note::On(64);
        pat2.cell_mut(1, 0).instrument = 1;

        let events = schedule_events(&song_from(2, vec![pat0, pat1, pat2],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1), OrderEntry::Pattern(2)]));

        let notes: Vec<_> = events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some(note),
            _ => None,
        }).collect();
        assert_eq!(notes, vec![64]);
    }

    // --- SetSpeed ---

    #[test]
    fn set_speed_does_not_change_row_timing() {
        let mut pat = Pattern::new(4, 1);
        pat.ticks_per_row = 0;
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(1, 0).effect = Effect::SetSpeed(3);
        pat.cell_mut(2, 0).note = Note::On(64);
        pat.cell_mut(2, 0).instrument = 1;

        let result = schedule_song(&one_channel_song(pat));

        let note_events: Vec<_> = result.events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some((note, e.time)),
            _ => None,
        }).collect();

        assert_eq!(note_events, vec![
            (60, MusicalTime::zero()),
            (64, time_at_row(2)),
        ]);
    }

    #[test]
    fn set_speed_still_emits_event() {
        let mut pat = Pattern::new(4, 1);
        pat.ticks_per_row = 0;
        pat.cell_mut(2, 0).effect = Effect::SetSpeed(3);

        let result = schedule_song(&one_channel_song(pat));
        let speed_events: Vec<_> = result.events.iter().filter(|e|
            matches!(e.payload, EventPayload::SetSpeed(_))
        ).collect();
        assert_eq!(speed_events.len(), 1);
    }

    // --- NoteDelay tests ---

    #[test]
    fn note_delay_offsets_note_event() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::NoteDelay(3);

        let events = schedule_events(&one_channel_song(pat));

        let note = events.iter().find(|e| matches!(e.payload, EventPayload::NoteOn { .. }));
        assert!(note.is_some());
        let expected = MusicalTime::zero().add_ticks(3, 24);
        assert_eq!(note.unwrap().time, expected);
    }

    #[test]
    fn note_delay_zero_plays_normally() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::NoteDelay(0);

        let events = schedule_events(&one_channel_song(pat));

        let note = events.iter().find(|e| matches!(e.payload, EventPayload::NoteOn { .. }));
        assert_eq!(note.unwrap().time, MusicalTime::zero());
    }

    #[test]
    fn note_delay_does_not_emit_effect_event() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::NoteDelay(3);

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].payload, EventPayload::NoteOn { .. }));
    }

    #[test]
    fn note_delay_also_delays_volume_command() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).volume = VolumeCommand::Volume(32);
        pat.cell_mut(0, 0).effect = Effect::NoteDelay(2);

        let events = schedule_events(&one_channel_song(pat));

        let expected = MusicalTime::zero().add_ticks(2, 24);
        for e in &events {
            assert_eq!(e.time, expected);
        }
    }

    // --- PatternDelay tests ---

    #[test]
    fn pattern_delay_adds_extra_rows_in_beat_space() {
        let mut pat = Pattern::new(2, 1);
        pat.cell_mut(0, 0).effect = Effect::PatternDelay(2);

        let result = schedule_song(&one_channel_song(pat));
        assert_eq!(result.total_time, MusicalTime::from_beats(1));
    }

    #[test]
    fn pattern_delay_shifts_subsequent_rows() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).effect = Effect::PatternDelay(1);
        pat.cell_mut(1, 0).note = Note::On(60);
        pat.cell_mut(1, 0).instrument = 1;

        let events = schedule_events(&one_channel_song(pat));

        let note = events.iter().find(|e| matches!(e.payload, EventPayload::NoteOn { .. }));
        assert_eq!(note.unwrap().time, time_at_row(2));
    }

    #[test]
    fn pattern_delay_does_not_emit_effect_event() {
        let mut pat = Pattern::new(2, 1);
        pat.cell_mut(0, 0).effect = Effect::PatternDelay(2);

        let events = schedule_events(&one_channel_song(pat));
        assert!(events.is_empty());
    }

    #[test]
    fn pattern_break_does_not_emit_effect_event() {
        let mut pat0 = Pattern::new(2, 1);
        pat0.cell_mut(0, 0).effect = Effect::PatternBreak(0);

        let song = one_channel_song(pat0);
        let events = schedule_events(&song);
        assert!(events.is_empty());
    }

    // --- time_for_track_clip_row tests ---

    #[test]
    fn time_for_row_single_occurrence() {
        let song = one_channel_song(Pattern::new(8, 1));
        let times = time_for_track_clip_row(&song.tracks[0], 0, 3, song.rows_per_beat);
        assert_eq!(times, vec![time_at_row(3)]);
    }

    #[test]
    fn time_for_row_repeated_pattern() {
        let song = song_from(1, vec![Pattern::new(4, 1)],
            vec![OrderEntry::Pattern(0), OrderEntry::Pattern(0)]);

        let times = time_for_track_clip_row(&song.tracks[0], 0, 0, song.rows_per_beat);
        assert_eq!(times.len(), 2);
        assert_eq!(times[0], time_at_row(0));
        assert_eq!(times[1], time_at_row(4));
    }

    #[test]
    fn time_for_row_out_of_range_row() {
        let song = one_channel_song(Pattern::new(4, 1));
        let times = time_for_track_clip_row(&song.tracks[0], 0, 100, song.rows_per_beat);
        assert!(times.is_empty());
    }

    // --- SeqEntry.length tests ---

    #[test]
    fn mute_truncated_entry_plays_shortened() {
        let mut pat = Pattern::new(8, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(4, 0).note = Note::On(64);
        pat.cell_mut(4, 0).instrument = 1;

        let mut song = Song::with_channels("test", 1);
        let machine_node = mb_ir::find_tracker_node(&song.graph);
        let mut track = mb_ir::Track::new(machine_node, 0, 1);
        track.clips.push(mb_ir::Clip::Pattern(pat));
        // Mute truncates the 8-row pattern to 4 rows
        track.sequence.push(mb_ir::SeqEntry {
            start: MusicalTime::zero(), clip_idx: 0, length: 4,
            termination: mb_ir::SeqTermination::Mute,
        });
        song.tracks = alloc::vec![track];

        let notes: Vec<_> = schedule_events(&song).into_iter()
            .filter(|e| matches!(e.payload, EventPayload::NoteOn { .. }))
            .collect();
        // Only the note at row 0 should play; row 4 is beyond truncated length
        assert_eq!(notes.len(), 1, "mute-truncated entry should only play rows 0-3");
        assert_eq!(notes[0].time, MusicalTime::zero());
    }

    #[test]
    fn length_truncates_pattern() {
        let mut pat = Pattern::new(8, 1);
        // Notes at rows 0, 2, 4, 6
        for r in [0, 2, 4, 6] {
            pat.cell_mut(r, 0).note = Note::On(60);
            pat.cell_mut(r, 0).instrument = 1;
        }

        let mut song = Song::with_channels("test", 1);
        let machine_node = mb_ir::find_tracker_node(&song.graph);
        let mut track = mb_ir::Track::new(machine_node, 0, 1);
        track.clips.push(mb_ir::Clip::Pattern(pat));
        // length=4 means only rows 0-3 should play (notes at 0 and 2)
        track.sequence.push(mb_ir::SeqEntry {
            start: MusicalTime::zero(), clip_idx: 0, length: 4,
            termination: mb_ir::SeqTermination::Natural,
        });
        song.tracks = alloc::vec![track];

        let notes: Vec<_> = schedule_events(&song).into_iter()
            .filter(|e| matches!(e.payload, EventPayload::NoteOn { .. }))
            .collect();
        assert_eq!(notes.len(), 2, "length=4 should only play rows 0-3 (2 notes)");
    }

    #[test]
    fn break_truncated_entry_plays_shortened() {
        let mut pat = Pattern::new(8, 1);
        for r in [0, 2, 4, 6] {
            pat.cell_mut(r, 0).note = Note::On(60);
            pat.cell_mut(r, 0).instrument = 1;
        }

        let mut song = Song::with_channels("test", 1);
        let machine_node = mb_ir::find_tracker_node(&song.graph);
        let mut track = mb_ir::Track::new(machine_node, 0, 1);
        track.clips.push(mb_ir::Clip::Pattern(pat));
        // Break truncates the 8-row pattern to 3 rows
        track.sequence.push(mb_ir::SeqEntry {
            start: MusicalTime::zero(), clip_idx: 0, length: 3,
            termination: mb_ir::SeqTermination::Break,
        });
        song.tracks = alloc::vec![track];

        let notes: Vec<_> = schedule_events(&song).into_iter()
            .filter(|e| matches!(e.payload, EventPayload::NoteOn { .. }))
            .collect();
        // Only rows 0 and 2 should play (row 4 is beyond length=3)
        assert_eq!(notes.len(), 2, "break-truncated entry should only play rows 0-2");
    }

    #[test]
    fn time_for_row_respects_entry_length() {
        let mut song = Song::with_channels("test", 1);
        let machine_node = mb_ir::find_tracker_node(&song.graph);
        let mut track = mb_ir::Track::new(machine_node, 0, 1);
        track.clips.push(mb_ir::Clip::Pattern(Pattern::new(8, 1)));
        // length=4: only rows 0-3 are valid
        track.sequence.push(mb_ir::SeqEntry {
            start: MusicalTime::zero(), clip_idx: 0, length: 4,
            termination: mb_ir::SeqTermination::Natural,
        });
        song.tracks = alloc::vec![track];

        // Row 3 is within length — should return a time
        let times = time_for_track_clip_row(&song.tracks[0], 0, 3, song.rows_per_beat);
        assert_eq!(times.len(), 1);

        // Row 4 is beyond length — should return empty
        let times = time_for_track_clip_row(&song.tracks[0], 0, 4, song.rows_per_beat);
        assert!(times.is_empty(), "row beyond entry.length should not be accessible");
    }
}
