//! Pattern-to-event scheduling.
//!
//! Walks a song's order list and patterns, producing a sorted Vec<Event>
//! that the engine can consume for playback.

use alloc::vec::Vec;
use mb_ir::{
    Cell, Effect, Event, EventPayload, EventTarget, MusicalTime, Note, OrderEntry, Song,
    VolumeCommand,
};

/// Result of scheduling a song: events and total length.
pub struct ScheduleResult {
    pub events: Vec<Event>,
    pub total_time: MusicalTime,
}

/// Resolve the order entry at `order_idx`, skipping Skip entries.
/// Returns the pattern index, or None if End or out of bounds.
fn resolve_order(song: &Song, order_idx: &mut usize) -> Option<u8> {
    while *order_idx < song.order.len() {
        match song.order[*order_idx] {
            OrderEntry::Pattern(idx) => return Some(idx),
            OrderEntry::Skip => *order_idx += 1,
            OrderEntry::End => return None,
        }
    }
    None
}

/// Flow control state extracted from a pattern row.
struct FlowControl {
    break_row: Option<u8>,
    jump_order: Option<u8>,
    new_speed: Option<u32>,
    pattern_delay: u8,
}

/// Scan a pattern row for flow control effects.
fn scan_flow_control(pattern: &mb_ir::Pattern, row: u16) -> FlowControl {
    let mut fc = FlowControl {
        break_row: None,
        jump_order: None,
        new_speed: None,
        pattern_delay: 0,
    };
    for ch in 0..pattern.channels {
        match pattern.cell(row, ch).effect {
            Effect::PatternBreak(r) => fc.break_row = Some(r),
            Effect::PositionJump(p) => fc.jump_order = Some(p),
            Effect::SetSpeed(s) if s > 0 => fc.new_speed = Some(s as u32),
            Effect::PatternDelay(d) => fc.pattern_delay = d,
            _ => {}
        }
    }
    fc
}

/// Schedule all events for a song by walking rows with flow control.
///
/// Row positions are in beat-space (speed-independent). Speed only affects
/// NoteDelay sub-beat offsets and SetSpeed events.
pub fn schedule_song(song: &Song) -> ScheduleResult {
    let mut events = Vec::new();
    let mut order_idx: usize = 0;
    let mut row: u16 = 0;
    let mut time = MusicalTime::zero();
    let mut speed: u32 = song.initial_speed as u32;
    let song_rpb = song.rows_per_beat as u32;

    // Loop detection: cap at 2x total rows across all patterns
    let max_rows: u64 = song.patterns.iter().map(|p| p.rows as u64).sum::<u64>() * 2 + 256;
    let mut rows_processed: u64 = 0;

    loop {
        let pat_idx = match resolve_order(song, &mut order_idx) {
            Some(idx) => idx,
            None => break,
        };
        let pattern = match song.patterns.get(pat_idx as usize) {
            Some(p) => p,
            None => break,
        };
        if row >= pattern.rows {
            row = 0;
        }

        let rpb = pattern.rows_per_beat.map_or(song_rpb, |r| r as u32);

        // Schedule events for this row (speed needed for NoteDelay computation)
        let eff_speed = effective_speed(pattern, speed);
        for ch in 0..pattern.channels {
            schedule_cell(pattern.cell(row, ch), time, ch, eff_speed, rpb, &mut events);
        }

        // Scan for flow control and speed changes
        let fc = scan_flow_control(pattern, row);
        if let Some(s) = fc.new_speed {
            speed = s;
        }

        // Advance time by 1 + pattern_delay rows in beat-space
        time = time.add_rows(1 + fc.pattern_delay as u32, rpb);
        rows_processed += 1;
        if rows_processed >= max_rows {
            break;
        }

        // Handle flow control: determine next row/order position
        match (fc.jump_order, fc.break_row) {
            (Some(pos), Some(r)) => {
                order_idx = pos as usize;
                row = r as u16;
            }
            (Some(pos), None) => {
                order_idx = pos as usize;
                row = 0;
            }
            (None, Some(r)) => {
                order_idx += 1;
                row = r as u16;
            }
            (None, None) => {
                row += 1;
                if row >= pattern.rows {
                    order_idx += 1;
                    row = 0;
                }
            }
        }
    }

    ScheduleResult { events, total_time: time }
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

/// Convert a single cell into events and append them to the output.
///
/// `speed` and `rpb` are needed for NoteDelay sub-beat computation:
/// ticks_per_beat = speed * rpb.
fn schedule_cell(
    cell: &Cell,
    time: MusicalTime,
    channel: u8,
    speed: u32,
    rpb: u32,
    events: &mut Vec<Event>,
) {
    let target = EventTarget::Channel(channel);
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
    schedule_volume_command(&cell.volume, note_time, channel, events);
    // Effect fires at row time (except NoteDelay/PatternDelay are consumed)
    schedule_effect(&cell.effect, time, channel, events);
}

/// Convert a volume column command into an event.
fn schedule_volume_command(
    vol: &VolumeCommand,
    time: MusicalTime,
    channel: u8,
    events: &mut Vec<Event>,
) {
    match vol {
        VolumeCommand::None => {}
        VolumeCommand::Volume(v) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::SetVolume(*v)),
            ));
        }
        // Other volume commands map to their effect equivalents
        VolumeCommand::Panning(p) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::SetPan(*p)),
            ));
        }
        VolumeCommand::TonePorta(v) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::TonePorta(*v)),
            ));
        }
        VolumeCommand::Vibrato(v) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::Vibrato { speed: 0, depth: *v }),
            ));
        }
        VolumeCommand::VolumeSlideDown(v) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::VolumeSlide(-(*v as i8))),
            ));
        }
        VolumeCommand::VolumeSlideUp(v) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::VolumeSlide(*v as i8)),
            ));
        }
        VolumeCommand::FineVolSlideDown(v) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::FineVolumeSlideDown(*v)),
            ));
        }
        VolumeCommand::FineVolSlideUp(v) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::FineVolumeSlideUp(*v)),
            ));
        }
        VolumeCommand::PortaDown(v) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::PortaDown(*v)),
            ));
        }
        VolumeCommand::PortaUp(v) => {
            events.push(Event::new(
                time,
                EventTarget::Channel(channel),
                EventPayload::Effect(Effect::PortaUp(*v)),
            ));
        }
    }
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
fn schedule_effect(effect: &Effect, time: MusicalTime, channel: u8, events: &mut Vec<Event>) {
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
                EventTarget::Channel(channel),
                EventPayload::Effect(*other),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::Pattern;

    /// Schedule and return just the events (convenience for tests).
    fn schedule_events(song: &Song) -> Vec<Event> {
        schedule_song(song).events
    }

    /// Build a minimal 1-channel song with a single pattern.
    fn one_channel_song(pattern: Pattern) -> Song {
        let mut song = Song::with_channels("test", 1);
        let idx = song.add_pattern(pattern);
        song.add_order(OrderEntry::Pattern(idx));
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
        assert_eq!(events[0].target, EventTarget::Channel(0));
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
        // Row 3 at rpb=4: beat 0, sub_beat = 3 * (720720/4) = 3 * 180180
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

        let mut song = Song::with_channels("test", 3);
        let idx = song.add_pattern(pat);
        song.add_order(OrderEntry::Pattern(idx));

        let events = schedule_events(&song);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].target, EventTarget::Channel(0));
        assert_eq!(events[1].target, EventTarget::Channel(2));
        // Both at time zero
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

        let mut song = Song::with_channels("test", 1);
        let idx0 = song.add_pattern(pat0);
        let idx1 = song.add_pattern(pat1);
        song.add_order(OrderEntry::Pattern(idx0));
        song.add_order(OrderEntry::Pattern(idx1));

        let events = schedule_events(&song);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time, MusicalTime::zero());
        // Pattern 0: 4 rows at rpb=4 = 1 beat
        assert_eq!(events[1].time, MusicalTime::from_beats(1));
    }

    #[test]
    fn repeated_pattern_in_order() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;

        let mut song = Song::with_channels("test", 1);
        let idx = song.add_pattern(pat);
        song.add_order(OrderEntry::Pattern(idx));
        song.add_order(OrderEntry::Pattern(idx));

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
        assert_eq!(events[0].target, EventTarget::Channel(0));
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
        // Same time
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

        let mut song = Song::with_channels("test", 1);
        let idx = song.add_pattern(pat);
        song.add_order(OrderEntry::Pattern(idx));
        song.add_order(OrderEntry::End);
        song.add_order(OrderEntry::Pattern(idx));

        let events = schedule_events(&song);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn order_skip_is_ignored() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;

        let mut song = Song::with_channels("test", 1);
        let idx = song.add_pattern(pat);
        song.add_order(OrderEntry::Pattern(idx));
        song.add_order(OrderEntry::Skip);
        song.add_order(OrderEntry::Pattern(idx));

        let events = schedule_events(&song);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time, MusicalTime::zero());
        assert_eq!(events[1].time, MusicalTime::from_beats(1));
    }

    #[test]
    fn total_time_matches_pattern_rows() {
        let pat = Pattern::new(4, 1); // 4 rows at rpb=4 = 1 beat
        let result = schedule_song(&one_channel_song(pat));
        assert_eq!(result.total_time, MusicalTime::from_beats(1));
    }

    #[test]
    fn total_time_sums_across_order() {
        let mut song = Song::with_channels("test", 1);
        let idx = song.add_pattern(Pattern::new(8, 1)); // 8 rows = 2 beats
        song.add_order(OrderEntry::Pattern(idx));
        song.add_order(OrderEntry::Pattern(idx));
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

        let mut song = Song::with_channels("test", 1);
        let idx0 = song.add_pattern(pat0);
        let idx1 = song.add_pattern(pat1);
        song.add_order(OrderEntry::Pattern(idx0));
        song.add_order(OrderEntry::Pattern(idx1));

        let result = schedule_song(&song);

        let notes: Vec<_> = result.events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some((note, e.time)),
            _ => None,
        }).collect();

        // Note 60 at row 0, note 64 at row 2 (break at row 1 = 2 rows played)
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

        let mut song = Song::with_channels("test", 1);
        let idx0 = song.add_pattern(pat0);
        let idx1 = song.add_pattern(pat1);
        song.add_order(OrderEntry::Pattern(idx0));
        song.add_order(OrderEntry::Pattern(idx1));

        let events = schedule_events(&song);

        let notes: Vec<_> = events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some((note, e.time)),
            _ => None,
        }).collect();

        // Break after row 0 (1 row of beat-space). Jump to row 2 of pat1.
        // Row 2 is reached after row 0, row 1 of pat1 → total 1 + 2 inner rows
        // But since we jump directly to row 2, the scheduler processes row 2
        // *next*, so it's at: time after 1 row (break) = time_at_row(1).
        // Then the scheduler doesn't process rows 0-1 of pat1, it starts at 2.
        // The note fires when the scheduler first visits that row.
        assert_eq!(notes, vec![(60, time_at_row(1))]);
    }

    #[test]
    fn pattern_break_total_time() {
        // Pat0: 4 rows, break at row 1 → only 2 rows play
        // Pat1: 4 rows → 4 rows. Total = 6 rows = 1.5 beats
        let mut pat0 = Pattern::new(4, 1);
        pat0.cell_mut(1, 0).effect = Effect::PatternBreak(0);

        let mut song = Song::with_channels("test", 1);
        let idx0 = song.add_pattern(pat0);
        let idx1 = song.add_pattern(Pattern::new(4, 1));
        song.add_order(OrderEntry::Pattern(idx0));
        song.add_order(OrderEntry::Pattern(idx1));

        let result = schedule_song(&song);
        // 2 + 4 = 6 rows at rpb=4 = 1 beat + 2 rows = beat 1, sub_beat = 2*180180
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

        let mut song = Song::with_channels("test", 1);
        let i0 = song.add_pattern(pat0);
        let i1 = song.add_pattern(pat1);
        let i2 = song.add_pattern(pat2);
        song.add_order(OrderEntry::Pattern(i0));
        song.add_order(OrderEntry::Pattern(i1));
        song.add_order(OrderEntry::Pattern(i2));

        let events = schedule_events(&song);

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

        // Should terminate (not hang). max_rows = 2*2+256 = 260
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

        let mut song = Song::with_channels("test", 2);
        let i0 = song.add_pattern(pat0);
        let i1 = song.add_pattern(pat1);
        let i2 = song.add_pattern(pat2);
        song.add_order(OrderEntry::Pattern(i0));
        song.add_order(OrderEntry::Pattern(i1));
        song.add_order(OrderEntry::Pattern(i2));

        let events = schedule_events(&song);

        let notes: Vec<_> = events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some(note),
            _ => None,
        }).collect();
        assert_eq!(notes, vec![64]);
    }

    // --- SetSpeed no longer changes row timing (rows are beat-positioned) ---

    #[test]
    fn set_speed_does_not_change_row_timing() {
        // In beat-space, rows are equidistant regardless of speed.
        // SetSpeed only affects per-tick effects and NoteDelay.
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

        // Row 0 at time 0, Row 2 at time_at_row(2) — speed doesn't affect positioning
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
        // 3 ticks delay at speed=6, rpb=4 → tpb=24, sub_per_tick=30030
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
        // 2 rows, row 0 has PatternDelay(2) → row 0 takes 3 rows worth of beat-space
        let mut pat = Pattern::new(2, 1);
        pat.cell_mut(0, 0).effect = Effect::PatternDelay(2);

        let result = schedule_song(&one_channel_song(pat));

        // Row 0: 1+2 = 3 rows, Row 1: 1 row → total 4 rows = 1 beat
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
        // Row 0: 1+1 = 2 rows of beat-space. Row 1 at row offset 2.
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

        let mut song = Song::with_channels("test", 1);
        let idx = song.add_pattern(pat0);
        song.add_order(OrderEntry::Pattern(idx));

        let events = schedule_events(&song);
        assert!(events.is_empty());
    }
}
