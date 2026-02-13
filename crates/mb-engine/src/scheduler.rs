//! Pattern-to-event scheduling.
//!
//! Walks a song's order list and patterns, producing a sorted Vec<Event>
//! that the engine can consume for playback.

use alloc::vec::Vec;
use mb_ir::{
    Cell, Effect, Event, EventPayload, EventTarget, Note, OrderEntry, Song, Timestamp,
    VolumeCommand,
};

/// Result of scheduling a song: events and total length.
pub struct ScheduleResult {
    pub events: Vec<Event>,
    pub total_ticks: u64,
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
    new_speed: Option<u64>,
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
            Effect::SetSpeed(s) if s > 0 => fc.new_speed = Some(s as u64),
            Effect::PatternDelay(d) => fc.pattern_delay = d,
            _ => {}
        }
    }
    fc
}

/// Schedule all events for a song by walking rows with flow control.
pub fn schedule_song(song: &Song) -> ScheduleResult {
    let mut events = Vec::new();
    let mut order_idx: usize = 0;
    let mut row: u16 = 0;
    let mut tick: u64 = 0;
    let mut speed: u64 = song.initial_speed as u64;

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

        // Schedule events for this row
        let time = Timestamp::from_ticks(tick);
        for ch in 0..pattern.channels {
            schedule_cell(pattern.cell(row, ch), time, ch, &mut events);
        }

        // Scan for flow control and speed changes
        let fc = scan_flow_control(pattern, row);
        if let Some(s) = fc.new_speed {
            speed = s;
        }

        // Advance tick by current row's speed (+ pattern delay)
        let base_speed = if pattern.ticks_per_row > 0 {
            pattern.ticks_per_row as u64
        } else {
            speed
        };
        tick += base_speed + (fc.pattern_delay as u64 * base_speed);
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

    ScheduleResult { events, total_ticks: tick }
}

/// Returns true if the effect is a tone portamento variant.
fn is_tone_porta(effect: &Effect) -> bool {
    matches!(effect, Effect::TonePorta(_) | Effect::TonePortaVolSlide(_))
}

/// Extract NoteDelay tick offset from a cell's effect.
fn note_delay_ticks(effect: &Effect) -> u64 {
    match effect {
        Effect::NoteDelay(d) if *d > 0 => *d as u64,
        _ => 0,
    }
}

/// Convert a single cell into events and append them to the output.
fn schedule_cell(cell: &Cell, time: Timestamp, channel: u8, events: &mut Vec<Event>) {
    let target = EventTarget::Channel(channel);
    let delay = note_delay_ticks(&cell.effect);
    let note_time = Timestamp::from_ticks(time.tick + delay);

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
    time: Timestamp,
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
fn schedule_effect(effect: &Effect, time: Timestamp, channel: u8, events: &mut Vec<Event>) {
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
        assert_eq!(events[0].time.tick, 0);
        assert_eq!(events[0].target, EventTarget::Channel(0));
        assert_eq!(
            events[0].payload,
            EventPayload::NoteOn { note: 60, velocity: 64, instrument: 1 }
        );
    }

    #[test]
    fn note_at_row_n_offset_by_ticks_per_row() {
        let mut pat = Pattern::new(8, 1);
        pat.cell_mut(3, 0).note = Note::On(48);
        pat.cell_mut(3, 0).instrument = 2;

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        // ticks_per_row defaults to 6, so row 3 = tick 18
        assert_eq!(events[0].time.tick, 18);
    }

    #[test]
    fn note_off_produces_note_off_event() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(1, 0).note = Note::Off;

        let events = schedule_events(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].time.tick, 6);
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
        // Both at tick 0
        assert_eq!(events[0].time.tick, 0);
        assert_eq!(events[1].time.tick, 0);
    }

    #[test]
    fn two_patterns_in_order_offsets_correctly() {
        // Pattern 0: 4 rows, note at row 0
        let mut pat0 = Pattern::new(4, 1);
        pat0.cell_mut(0, 0).note = Note::On(60);
        pat0.cell_mut(0, 0).instrument = 1;

        // Pattern 1: 4 rows, note at row 0
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
        assert_eq!(events[0].time.tick, 0);
        // Pattern 0 has 4 rows * 6 ticks = 24 ticks, so pattern 1 starts at tick 24
        assert_eq!(events[1].time.tick, 24);
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
        assert_eq!(events[0].time.tick, 0);
        assert_eq!(events[1].time.tick, 24); // 4 rows * 6 ticks
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
        // NoteOn first, then effect
        assert!(matches!(events[0].payload, EventPayload::NoteOn { .. }));
        assert!(matches!(events[1].payload, EventPayload::Effect(_)));
        // Same tick
        assert_eq!(events[0].time.tick, events[1].time.tick);
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
        song.add_order(OrderEntry::Pattern(idx)); // should not be reached

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
        assert_eq!(events[0].time.tick, 0);
        assert_eq!(events[1].time.tick, 24);
    }

    #[test]
    fn total_ticks_matches_pattern_rows() {
        let pat = Pattern::new(4, 1); // 4 rows * 6 ticks_per_row = 24
        let result = schedule_song(&one_channel_song(pat));
        assert_eq!(result.total_ticks, 24);
    }

    #[test]
    fn total_ticks_sums_across_order() {
        let mut song = Song::with_channels("test", 1);
        let idx = song.add_pattern(Pattern::new(8, 1)); // 8 * 6 = 48
        song.add_order(OrderEntry::Pattern(idx));
        song.add_order(OrderEntry::Pattern(idx));
        let result = schedule_song(&song);
        assert_eq!(result.total_ticks, 96); // 48 * 2
    }

    #[test]
    fn tone_porta_with_note_emits_porta_target() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::TonePorta(8);

        let events = schedule_events(&one_channel_song(pat));

        // Should emit PortaTarget + Effect, NOT NoteOn + Effect
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
        // Pattern 0: 4 rows, break at row 1 → should skip rows 2-3
        let mut pat0 = Pattern::new(4, 1);
        pat0.cell_mut(0, 0).note = Note::On(60);
        pat0.cell_mut(0, 0).instrument = 1;
        pat0.cell_mut(1, 0).effect = Effect::PatternBreak(0);

        // Pattern 1: 4 rows, note at row 0
        let mut pat1 = Pattern::new(4, 1);
        pat1.cell_mut(0, 0).note = Note::On(64);
        pat1.cell_mut(0, 0).instrument = 1;

        let mut song = Song::with_channels("test", 1);
        let idx0 = song.add_pattern(pat0);
        let idx1 = song.add_pattern(pat1);
        song.add_order(OrderEntry::Pattern(idx0));
        song.add_order(OrderEntry::Pattern(idx1));

        let result = schedule_song(&song);

        // Filter to NoteOn events only (PatternBreak also generates an event)
        let notes: Vec<_> = result.events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some((note, e.time.tick)),
            _ => None,
        }).collect();

        // Note 60 at tick 0, note 64 at tick 12 (2 rows * 6 ticks)
        assert_eq!(notes, vec![(60, 0), (64, 12)]);
    }

    #[test]
    fn pattern_break_to_specific_row() {
        // Pattern 0: break at row 0 → go to row 2 of pattern 1
        let mut pat0 = Pattern::new(2, 1);
        pat0.cell_mut(0, 0).effect = Effect::PatternBreak(2);

        // Pattern 1: 4 rows, note at row 2
        let mut pat1 = Pattern::new(4, 1);
        pat1.cell_mut(2, 0).note = Note::On(60);
        pat1.cell_mut(2, 0).instrument = 1;

        let mut song = Song::with_channels("test", 1);
        let idx0 = song.add_pattern(pat0);
        let idx1 = song.add_pattern(pat1);
        song.add_order(OrderEntry::Pattern(idx0));
        song.add_order(OrderEntry::Pattern(idx1));

        let events = schedule_events(&song);

        // Filter to NoteOn events (PatternBreak also generates an effect event)
        let notes: Vec<_> = events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some((note, e.time.tick)),
            _ => None,
        }).collect();

        // Break fires after row 0 of pat0 (1 row = 6 ticks)
        // Jumps directly to row 2 of pat1 → note at tick 6
        assert_eq!(notes, vec![(60, 6)]);
    }

    #[test]
    fn pattern_break_total_ticks() {
        // Pat0: 4 rows, break at row 1 → only 2 rows play (12 ticks)
        // Pat1: 4 rows → 24 ticks. Total = 36
        let mut pat0 = Pattern::new(4, 1);
        pat0.cell_mut(1, 0).effect = Effect::PatternBreak(0);

        let mut song = Song::with_channels("test", 1);
        let idx0 = song.add_pattern(pat0);
        let idx1 = song.add_pattern(Pattern::new(4, 1));
        song.add_order(OrderEntry::Pattern(idx0));
        song.add_order(OrderEntry::Pattern(idx1));

        let result = schedule_song(&song);
        assert_eq!(result.total_ticks, 36);
    }

    // --- PositionJump tests ---

    #[test]
    fn position_jump_to_later_order() {
        // 3 patterns in order. Pat0 jumps to order 2 (pat2), skipping pat1.
        let mut pat0 = Pattern::new(2, 1);
        pat0.cell_mut(0, 0).note = Note::On(60);
        pat0.cell_mut(0, 0).instrument = 1;
        pat0.cell_mut(0, 0).effect = Effect::PositionJump(2);

        let mut pat1 = Pattern::new(2, 1);
        pat1.cell_mut(0, 0).note = Note::On(62); // should be skipped
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

        // Should have notes 60 (tick 0) and 64 (after 1 row = tick 6), no note 62
        let notes: Vec<_> = events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some(note),
            _ => None,
        }).collect();
        assert_eq!(notes, vec![60, 64]);
        assert_eq!(events[0].time.tick, 0);
    }

    #[test]
    fn position_jump_backwards_terminates() {
        // Jump back to order 0 creates a loop — should be capped by max_rows
        let mut pat = Pattern::new(2, 1);
        pat.cell_mut(1, 0).effect = Effect::PositionJump(0);

        let result = schedule_song(&one_channel_song(pat));

        // Should terminate (not hang). The loop cap is 2*2 + 256 = 260 rows.
        // Each iteration plays 2 rows, so ~130 iterations * 2 rows * 6 ticks
        assert!(result.total_ticks > 0);
        assert!(result.total_ticks <= 260 * 6);
    }

    // --- Combined PatternBreak + PositionJump ---

    #[test]
    fn position_jump_with_pattern_break() {
        // Jump to order 2, start at row 1
        let mut pat0 = Pattern::new(2, 2);
        pat0.cell_mut(0, 0).effect = Effect::PositionJump(2);
        pat0.cell_mut(0, 1).effect = Effect::PatternBreak(1);

        let mut pat1 = Pattern::new(4, 2);
        pat1.cell_mut(0, 0).note = Note::On(62); // skipped
        pat1.cell_mut(0, 0).instrument = 1;

        let mut pat2 = Pattern::new(4, 2);
        pat2.cell_mut(1, 0).note = Note::On(64); // row 1 = target
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
        // Should skip pat1 entirely, land on pat2 row 1
        assert_eq!(notes, vec![64]);
    }

    // --- SetSpeed affects scheduling ---

    #[test]
    fn set_speed_changes_subsequent_row_timing() {
        // Speed 6 (default), then SetSpeed(3) at row 1
        // ticks_per_row=0 so global speed applies
        let mut pat = Pattern::new(4, 1);
        pat.ticks_per_row = 0;
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(1, 0).effect = Effect::SetSpeed(3);
        pat.cell_mut(2, 0).note = Note::On(64);
        pat.cell_mut(2, 0).instrument = 1;

        let result = schedule_song(&one_channel_song(pat));

        let note_events: Vec<_> = result.events.iter().filter_map(|e| match e.payload {
            EventPayload::NoteOn { note, .. } => Some((note, e.time.tick)),
            _ => None,
        }).collect();

        // Row 0: tick 0 (speed=6), Row 1: tick 6 (speed→3), Row 2: tick 9
        assert_eq!(note_events, vec![(60, 0), (64, 9)]);
    }

    #[test]
    fn set_speed_affects_total_ticks() {
        // 4 rows, speed changes from 6 to 3 at row 2
        // ticks_per_row=0 so global speed applies
        // Row 0: 6, Row 1: 6, Row 2 (speed→3): 3, Row 3: 3 → total 18
        let mut pat = Pattern::new(4, 1);
        pat.ticks_per_row = 0;
        pat.cell_mut(2, 0).effect = Effect::SetSpeed(3);

        let result = schedule_song(&one_channel_song(pat));
        assert_eq!(result.total_ticks, 18);
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
        // Note at row 0 delayed by 3 ticks → tick 3
        assert_eq!(note.unwrap().time.tick, 3);
    }

    #[test]
    fn note_delay_zero_plays_normally() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::NoteDelay(0);

        let events = schedule_events(&one_channel_song(pat));

        let note = events.iter().find(|e| matches!(e.payload, EventPayload::NoteOn { .. }));
        assert_eq!(note.unwrap().time.tick, 0);
    }

    #[test]
    fn note_delay_does_not_emit_effect_event() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;
        pat.cell_mut(0, 0).effect = Effect::NoteDelay(3);

        let events = schedule_events(&one_channel_song(pat));

        // Should only have the NoteOn event, no NoteDelay effect event
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

        // Both events should be at tick 2
        for e in &events {
            assert_eq!(e.time.tick, 2);
        }
    }

    // --- PatternDelay tests ---

    #[test]
    fn pattern_delay_adds_extra_ticks() {
        // 2 rows, row 0 has PatternDelay(2) → row 0 takes 3x normal ticks
        let mut pat = Pattern::new(2, 1);
        pat.cell_mut(0, 0).effect = Effect::PatternDelay(2);

        let result = schedule_song(&one_channel_song(pat));

        // Row 0: 6 + 2*6 = 18 ticks, Row 1: 6 ticks → total 24
        assert_eq!(result.total_ticks, 24);
    }

    #[test]
    fn pattern_delay_shifts_subsequent_rows() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).effect = Effect::PatternDelay(1);
        pat.cell_mut(1, 0).note = Note::On(60);
        pat.cell_mut(1, 0).instrument = 1;

        let events = schedule_events(&one_channel_song(pat));

        let note = events.iter().find(|e| matches!(e.payload, EventPayload::NoteOn { .. }));
        // Row 0: 6 + 1*6 = 12 ticks. Row 1 note at tick 12.
        assert_eq!(note.unwrap().time.tick, 12);
    }

    #[test]
    fn pattern_delay_does_not_emit_effect_event() {
        let mut pat = Pattern::new(2, 1);
        pat.cell_mut(0, 0).effect = Effect::PatternDelay(2);

        let events = schedule_events(&one_channel_song(pat));

        // No events should be emitted for PatternDelay
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

        // PatternBreak should be consumed by scheduler
        assert!(events.is_empty());
    }
}
