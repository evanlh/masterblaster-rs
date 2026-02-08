//! Pattern-to-event scheduling.
//!
//! Walks a song's order list and patterns, producing a sorted Vec<Event>
//! that the engine can consume for playback.

use alloc::vec::Vec;
use mb_ir::{
    Cell, Effect, Event, EventPayload, EventTarget, Note, OrderEntry, Song, Timestamp,
    VolumeCommand,
};

/// Schedule all events for a song by walking its order list and patterns.
pub fn schedule_song(song: &Song) -> Vec<Event> {
    let mut events = Vec::new();
    let mut tick: u64 = 0;
    let speed = song.initial_speed as u64;

    for entry in &song.order {
        match entry {
            OrderEntry::Pattern(idx) => {
                if let Some(pattern) = song.patterns.get(*idx as usize) {
                    let ticks_per_row = pattern.ticks_per_row as u64;
                    let row_speed = if ticks_per_row > 0 { ticks_per_row } else { speed };

                    for row in 0..pattern.rows {
                        let row_tick = tick + row as u64 * row_speed;
                        let time = Timestamp::from_ticks(row_tick);

                        for ch in 0..pattern.channels {
                            schedule_cell(pattern.cell(row, ch), time, ch, &mut events);
                        }
                    }

                    tick += pattern.rows as u64 * row_speed;
                }
            }
            OrderEntry::Skip => {}
            OrderEntry::End => break,
        }
    }

    events
}

/// Convert a single cell into events and append them to the output.
fn schedule_cell(cell: &Cell, time: Timestamp, channel: u8, events: &mut Vec<Event>) {
    let target = EventTarget::Channel(channel);

    match cell.note {
        Note::On(note) => {
            events.push(Event::new(
                time,
                target,
                EventPayload::NoteOn {
                    note,
                    velocity: 64,
                    instrument: cell.instrument,
                },
            ));
        }
        Note::Off | Note::Fade => {
            events.push(Event::new(
                time,
                target,
                EventPayload::NoteOff { note: 0 },
            ));
        }
        Note::None => {}
    }

    schedule_volume_command(&cell.volume, time, channel, events);
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

/// Convert an effect command into an event, routing tempo/speed to Global.
fn schedule_effect(effect: &Effect, time: Timestamp, channel: u8, events: &mut Vec<Event>) {
    match effect {
        Effect::None => {}
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
        let events = schedule_song(&song);
        assert!(events.is_empty());
    }

    #[test]
    fn single_note_at_row_zero() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).note = Note::On(60);
        pat.cell_mut(0, 0).instrument = 1;

        let events = schedule_song(&one_channel_song(pat));

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

        let events = schedule_song(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        // ticks_per_row defaults to 6, so row 3 = tick 18
        assert_eq!(events[0].time.tick, 18);
    }

    #[test]
    fn note_off_produces_note_off_event() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(1, 0).note = Note::Off;

        let events = schedule_song(&one_channel_song(pat));

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

        let events = schedule_song(&song);

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

        let events = schedule_song(&song);

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

        let events = schedule_song(&song);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time.tick, 0);
        assert_eq!(events[1].time.tick, 24); // 4 rows * 6 ticks
    }

    #[test]
    fn set_tempo_routes_to_global() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(1, 0).effect = Effect::SetTempo(140);

        let events = schedule_song(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].target, EventTarget::Global);
        assert_eq!(events[0].payload, EventPayload::SetTempo(14000));
    }

    #[test]
    fn set_speed_routes_to_global() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).effect = Effect::SetSpeed(3);

        let events = schedule_song(&one_channel_song(pat));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].target, EventTarget::Global);
        assert_eq!(events[0].payload, EventPayload::SetSpeed(3));
    }

    #[test]
    fn effect_routes_to_channel() {
        let mut pat = Pattern::new(4, 1);
        pat.cell_mut(0, 0).effect = Effect::VolumeSlide(4);

        let events = schedule_song(&one_channel_song(pat));

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

        let events = schedule_song(&one_channel_song(pat));

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

        let events = schedule_song(&one_channel_song(pat));

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

        let events = schedule_song(&song);

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

        let events = schedule_song(&song);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].time.tick, 0);
        assert_eq!(events[1].time.tick, 24);
    }
}
