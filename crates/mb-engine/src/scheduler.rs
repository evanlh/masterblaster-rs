//! Pattern-to-event scheduling.
//!
//! Walks a song's per-track clips and sequences, producing a sorted Vec<Event>
//! that the engine can consume for playback.

use alloc::vec::Vec;
use mb_ir::{
    Cell, Effect, Event, EventPayload, EventTarget, MusicalTime, Note, Song,
    Track, VolumeCommand,
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
///
/// Groups are processed together (shared flow control).
/// Ungrouped tracks are processed independently.
pub fn schedule_song(song: &Song) -> ScheduleResult {
    let mut events = Vec::new();

    let groups = collect_groups(&song.tracks);
    let ungrouped: Vec<usize> = song.tracks.iter().enumerate()
        .filter(|(_, t)| t.group.is_none())
        .map(|(i, _)| i)
        .collect();

    let mut max_time = MusicalTime::zero();

    for group_id in groups {
        let group_tracks: Vec<usize> = song.tracks.iter().enumerate()
            .filter(|(_, t)| t.group == Some(group_id))
            .map(|(i, _)| i)
            .collect();
        let t = schedule_group(&group_tracks, song, &mut events);
        if t > max_time { max_time = t; }
    }

    for &track_idx in &ungrouped {
        let t = schedule_ungrouped_track(&song.tracks[track_idx], track_idx, song, &mut events);
        if t > max_time { max_time = t; }
    }

    ScheduleResult { events, total_time: max_time }
}

/// Collect unique group IDs from tracks (sorted, deduplicated).
fn collect_groups(tracks: &[Track]) -> Vec<u16> {
    let mut groups: Vec<u16> = tracks.iter()
        .filter_map(|t| t.group)
        .collect();
    groups.sort_unstable();
    groups.dedup();
    groups
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
pub fn schedule_cell(
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

/// Resolve the channel index for a track at a given index in the song.
pub fn track_channel_index_from_song(track_idx: u16, song: &Song) -> Option<u8> {
    song.tracks.get(track_idx as usize).and_then(|t| track_channel_index(t, song))
}

/// Resolve the channel index for a track by finding its TrackerChannel node.
fn track_channel_index(track: &Track, song: &Song) -> Option<u8> {
    song.graph.node(track.target).and_then(|n| {
        if let mb_ir::NodeType::TrackerChannel { index } = &n.node_type {
            Some(*index)
        } else {
            None
        }
    })
}

/// Schedule events for a group of tracks that share identical sequences.
///
/// Walks position-by-position, row-by-row, gathering flow control across
/// all tracks in the group.
fn schedule_group(
    track_indices: &[usize],
    song: &Song,
    events: &mut Vec<Event>,
) -> MusicalTime {
    if track_indices.is_empty() {
        return MusicalTime::zero();
    }

    let seq = &song.tracks[track_indices[0]].sequence;
    if seq.is_empty() {
        return MusicalTime::zero();
    }

    // Build (track_index, channel_index) pairs
    let channels: Vec<(usize, u8)> = track_indices.iter()
        .filter_map(|&ti| track_channel_index(&song.tracks[ti], song).map(|ch| (ti, ch)))
        .collect();

    let song_rpb = song.rows_per_beat as u32;
    let mut speed: u32 = song.initial_speed as u32;
    let mut seq_idx: usize = 0;
    let mut row: u16 = 0;
    let mut time = MusicalTime::zero();

    let max_rows = compute_group_max_rows(&channels, song);
    let mut rows_processed: u64 = 0;

    loop {
        if seq_idx >= seq.len() { break; }
        let clip_idx = seq[seq_idx].clip_idx as usize;

        let rep_clip = match get_track_clip(&song.tracks[channels[0].0], clip_idx) {
            Some(p) => p,
            None => break,
        };
        let num_rows = rep_clip.rows;
        if row >= num_rows { row = 0; }
        let rpb = rep_clip.rows_per_beat.map_or(song_rpb, |r| r as u32);

        for &(ti, ch) in &channels {
            let clip = match get_track_clip(&song.tracks[ti], clip_idx) {
                Some(p) => p,
                None => continue,
            };
            let eff_speed = effective_speed(clip, speed);
            schedule_cell(clip.cell(row, 0), time, ch, eff_speed, rpb, events);
        }

        let fc = scan_group_flow_control(&channels, song, clip_idx, row);
        if let Some(s) = fc.new_speed { speed = s; }

        time = time.add_rows(1 + fc.pattern_delay as u32, rpb);
        rows_processed += 1;
        if rows_processed >= max_rows { break; }

        match (fc.jump_order, fc.break_row) {
            (Some(pos), Some(r)) => { seq_idx = pos as usize; row = r as u16; }
            (Some(pos), None) => { seq_idx = pos as usize; row = 0; }
            (None, Some(r)) => { seq_idx += 1; row = r as u16; }
            (None, None) => {
                row += 1;
                if row >= num_rows { seq_idx += 1; row = 0; }
            }
        }
    }

    time
}

/// Get the Pattern from a track's clip pool.
fn get_track_clip(track: &Track, clip_idx: usize) -> Option<&mb_ir::Pattern> {
    track.clips.get(clip_idx).and_then(|c| c.pattern())
}

/// Compute max rows for loop detection across a group of tracks.
fn compute_group_max_rows(channels: &[(usize, u8)], song: &Song) -> u64 {
    let total: u64 = channels.iter()
        .flat_map(|(ti, _)| song.tracks[*ti].clips.iter())
        .filter_map(|c| c.pattern().map(|p| p.rows as u64))
        .sum();
    total * 2 + 256
}

/// Scan flow control effects across all tracks in a group at a given row.
fn scan_group_flow_control(
    channels: &[(usize, u8)],
    song: &Song,
    clip_idx: usize,
    row: u16,
) -> FlowControl {
    let mut fc = FlowControl {
        break_row: None,
        jump_order: None,
        new_speed: None,
        pattern_delay: 0,
    };
    for &(ti, _) in channels {
        let Some(clip) = get_track_clip(&song.tracks[ti], clip_idx) else { continue };
        if row >= clip.rows { continue; }
        match clip.cell(row, 0).effect {
            Effect::PatternBreak(r) => fc.break_row = Some(r),
            Effect::PositionJump(p) => fc.jump_order = Some(p),
            Effect::SetSpeed(s) if s > 0 => fc.new_speed = Some(s as u32),
            Effect::PatternDelay(d) => fc.pattern_delay = d,
            _ => {}
        }
    }
    fc
}

/// Schedule events for a single ungrouped track.
fn schedule_ungrouped_track(
    track: &Track,
    _track_idx: usize,
    song: &Song,
    events: &mut Vec<Event>,
) -> MusicalTime {
    let ch = match track_channel_index(track, song) {
        Some(ch) => ch,
        None => return MusicalTime::zero(),
    };

    let song_rpb = song.rows_per_beat as u32;
    let mut speed: u32 = song.initial_speed as u32;
    let mut time = MusicalTime::zero();

    for entry in &track.sequence {
        let clip = match track.clips.get(entry.clip_idx as usize).and_then(|c| c.pattern()) {
            Some(p) => p,
            None => continue,
        };
        let rpb = clip.rows_per_beat.map_or(song_rpb, |r| r as u32);
        for row in 0..clip.rows {
            let eff_speed = effective_speed(clip, speed);
            schedule_cell(clip.cell(row, 0), time, ch, eff_speed, rpb, events);
            if let Effect::SetSpeed(s) = clip.cell(row, 0).effect {
                if s > 0 { speed = s as u32; }
            }
            time = time.add_rows(1, rpb);
        }
    }

    time
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
        .filter(|e| e.clip_idx == clip_idx)
        .filter_map(|e| {
            let clip = track.clips.get(e.clip_idx as usize)?.pattern()?;
            if row >= clip.rows { return None; }
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
        assert_eq!(events[0].target, EventTarget::Channel(0));
        assert_eq!(events[1].target, EventTarget::Channel(2));
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
}
