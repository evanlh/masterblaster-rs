//! Song structure and sequencing types.

use alloc::format;
use alloc::vec::Vec;
use arrayvec::ArrayString;

use crate::graph::{AudioGraph, NodeId, NodeType};
use crate::instrument::Instrument;
use crate::musical_time::MusicalTime;
use crate::pattern::Pattern;
use crate::sample::Sample;

/// A complete song.
#[derive(Clone, Debug)]
pub struct Song {
    /// Song title
    pub title: ArrayString<32>,
    /// Initial tempo in BPM (32-255 typical)
    pub initial_tempo: u8,
    /// Initial speed (ticks per row, 1-31)
    pub initial_speed: u8,
    /// Rows per beat (default 4: 4 rows = 1 beat)
    pub rows_per_beat: u8,
    /// Global volume (0-64)
    pub global_volume: u8,
    /// Instruments
    pub instruments: Vec<Instrument>,
    /// Samples
    pub samples: Vec<Sample>,
    /// Per-channel settings
    pub channels: Vec<ChannelSettings>,
    /// Audio routing graph
    pub graph: AudioGraph,
    /// Tracks (per-track sequencing)
    pub tracks: Vec<Track>,
}

impl Default for Song {
    fn default() -> Self {
        Self {
            title: ArrayString::new(),
            initial_tempo: 125,
            initial_speed: 6,
            rows_per_beat: 4,
            global_volume: 64,
            instruments: Vec::new(),
            samples: Vec::new(),
            channels: Vec::new(),
            graph: AudioGraph::with_master(),
            tracks: Vec::new(),
        }
    }
}

impl Song {
    /// Create a new empty song.
    pub fn new(title: &str) -> Self {
        let mut song = Self::default();
        let _ = song.title.try_push_str(title);
        song
    }

    /// Create a song with a given number of channels (for tracker formats).
    ///
    /// Graph: Chan0→AmigaFilter→Master, Chan1→AmigaFilter→Master, ...
    pub fn with_channels(title: &str, num_channels: u8) -> Self {
        use crate::graph::{NodeType, Parameter};

        let mut song = Self::new(title);

        // Insert Amiga filter between channels and master
        let filter_id = song
            .graph
            .add_node(NodeType::BuzzMachine { machine_name: alloc::string::String::from("Amiga Filter") });
        song.graph.node_mut(filter_id).unwrap().parameters.push(
            Parameter::new(0, "Cutoff", 1000, 22050, 4410),
        );
        song.graph.connect(filter_id, 0); // filter → master

        for i in 0..num_channels {
            song.channels.push(ChannelSettings {
                // Classic Amiga panning: L R R L pattern
                initial_pan: if i % 4 == 0 || i % 4 == 3 { -64 } else { 64 },
                initial_vol: 64,
                muted: false,
            });

            let node_id = song.graph.add_node(NodeType::TrackerChannel { index: i });
            song.graph.connect(node_id, filter_id); // channel → filter
        }

        song
    }

    /// Compute total song time from track sequences.
    pub fn total_time(&self) -> MusicalTime {
        self.tracks.iter()
            .filter_map(|track| track_end_time(track, self.rows_per_beat))
            .max()
            .unwrap_or(MusicalTime::zero())
    }
}

/// An entry in a legacy order list (used during format parsing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderEntry {
    /// Play pattern with this index
    Pattern(u8),
    /// Skip marker (+++), continue to next
    Skip,
    /// End of song marker (---)
    End,
}

/// Per-channel settings.
#[derive(Clone, Copy, Debug)]
pub struct ChannelSettings {
    /// Initial panning (-64 to +64, 0 = center)
    pub initial_pan: i8,
    /// Initial volume (0-64)
    pub initial_vol: u8,
    /// Is the channel muted?
    pub muted: bool,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            initial_pan: 0,
            initial_vol: 64,
            muted: false,
        }
    }
}

/// A track in the timeline — owns a clip pool and a playback sequence.
#[derive(Clone, Debug)]
pub struct Track {
    /// Which graph node this track controls
    pub target: NodeId,
    /// Name of the track
    pub name: ArrayString<32>,
    /// Pool of clips owned by this track
    pub clips: Vec<Clip>,
    /// Playback order (which clip to play when)
    pub sequence: Vec<SeqEntry>,
    /// UI grouping tag — tracks with the same group share a sequence and
    /// are displayed together in the pattern editor. `None` = ungrouped.
    pub group: Option<u16>,
}

impl Track {
    /// Create a new empty track targeting a node.
    pub fn new(target: NodeId, name: &str) -> Self {
        let mut track_name = ArrayString::new();
        let _ = track_name.try_push_str(name);
        Self {
            target,
            name: track_name,
            clips: Vec::new(),
            sequence: Vec::new(),
            group: None,
        }
    }
}

/// A clip in a track's pool.
#[derive(Clone, Debug)]
pub enum Clip {
    /// A single-column pattern (one channel of note data).
    Pattern(Pattern),
    // Automation variant deferred
}

impl Clip {
    /// Get the pattern if this is a Pattern clip.
    pub fn pattern(&self) -> Option<&Pattern> {
        match self {
            Clip::Pattern(p) => Some(p),
        }
    }

    /// Get a mutable reference to the pattern if this is a Pattern clip.
    pub fn pattern_mut(&mut self) -> Option<&mut Pattern> {
        match self {
            Clip::Pattern(p) => Some(p),
        }
    }
}

/// An entry in a track's sequence (playback order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeqEntry {
    /// When this clip starts playing
    pub start: MusicalTime,
    /// Index into the track's `clips` pool
    pub clip_idx: u16,
}

// --- Track building from legacy format data ---

/// Build per-track clips + sequences from multi-channel patterns and an order list.
///
/// Each channel becomes a track with single-column patterns extracted
/// from the multi-channel originals. All tracks get `group: Some(0)`.
pub fn build_tracks(
    song: &mut Song,
    patterns: &[Pattern],
    order: &[OrderEntry],
) {
    let num_channels = song.channels.len();
    if num_channels == 0 {
        return;
    }

    let channel_nodes: Vec<NodeId> = (0..num_channels)
        .map(|i| find_channel_node(&song.graph, i as u8))
        .collect();

    let mut tracks: Vec<Track> = (0..num_channels)
        .map(|i| {
            let mut t = Track::new(channel_nodes[i], &format_channel_name(i));
            t.group = Some(0);
            t
        })
        .collect();

    for pattern in patterns {
        for (ch, track) in tracks.iter_mut().enumerate() {
            let clip = extract_single_column(pattern, ch as u8);
            track.clips.push(Clip::Pattern(clip));
        }
    }

    let sequence = build_sequence_from_order(order, patterns, song.rows_per_beat);
    for track in &mut tracks {
        track.sequence = sequence.clone();
    }

    song.tracks = tracks;
}

/// Find the NodeId for TrackerChannel with the given index.
fn find_channel_node(graph: &AudioGraph, index: u8) -> NodeId {
    graph.nodes.iter()
        .position(|n| matches!(&n.node_type, NodeType::TrackerChannel { index: i } if *i == index))
        .unwrap_or(0) as NodeId
}

/// Format a channel name like "Ch 1", "Ch 2", etc.
fn format_channel_name(index: usize) -> ArrayString<32> {
    let mut name = ArrayString::new();
    let _ = name.try_push_str(&format!("Ch {}", index + 1));
    name
}

/// Extract a single channel column from a multi-channel pattern.
fn extract_single_column(pattern: &Pattern, channel: u8) -> Pattern {
    let mut single = Pattern::new(pattern.rows, 1);
    single.ticks_per_row = pattern.ticks_per_row;
    single.rows_per_beat = pattern.rows_per_beat;
    for row in 0..pattern.rows {
        if (channel as u8) < pattern.channels {
            *single.cell_mut(row, 0) = *pattern.cell(row, channel);
        }
    }
    single
}

/// Build a sequence from a legacy order list, computing start times.
fn build_sequence_from_order(
    order: &[OrderEntry],
    patterns: &[Pattern],
    song_rpb: u8,
) -> Vec<SeqEntry> {
    let rpb = song_rpb as u32;
    let mut sequence = Vec::new();
    let mut time = MusicalTime::zero();

    for entry in order {
        match entry {
            OrderEntry::Pattern(idx) => {
                sequence.push(SeqEntry { start: time, clip_idx: *idx as u16 });
                if let Some(pattern) = patterns.get(*idx as usize) {
                    let pat_rpb = pattern.rows_per_beat.map_or(rpb, |r| r as u32);
                    time = time.add_rows(pattern.rows as u32, pat_rpb);
                }
            }
            OrderEntry::Skip => {}
            OrderEntry::End => break,
        }
    }

    sequence
}

/// Compute the end time for a track (time after its last clip finishes).
fn track_end_time(track: &Track, song_rpb: u8) -> Option<MusicalTime> {
    let last = track.sequence.last()?;
    let clip = track.clips.get(last.clip_idx as usize)?;
    let pattern = clip.pattern()?;
    let rpb = pattern.rows_per_beat.map_or(song_rpb as u32, |r| r as u32);
    Some(last.start.add_rows(pattern.rows as u32, rpb))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_song() -> Song {
        let mut song = Song::with_channels("test", 4);

        let mut pat0 = Pattern::new(4, 4);
        pat0.cell_mut(0, 0).note = crate::pattern::Note::On(48);
        pat0.cell_mut(0, 0).instrument = 1;
        pat0.cell_mut(0, 2).note = crate::pattern::Note::On(60);
        pat0.cell_mut(0, 2).instrument = 2;
        pat0.cell_mut(3, 1).note = crate::pattern::Note::Off;

        let mut pat1 = Pattern::new(8, 4);
        pat1.cell_mut(0, 3).note = crate::pattern::Note::On(72);
        pat1.cell_mut(0, 3).instrument = 3;

        let patterns = vec![pat0, pat1];
        let order = vec![OrderEntry::Pattern(0), OrderEntry::Pattern(1)];
        build_tracks(&mut song, &patterns, &order);

        song
    }

    #[test]
    fn build_tracks_creates_one_track_per_channel() {
        let song = make_test_song();
        assert_eq!(song.tracks.len(), 4);
    }

    #[test]
    fn build_tracks_all_grouped() {
        let song = make_test_song();
        for track in &song.tracks {
            assert_eq!(track.group, Some(0));
        }
    }

    #[test]
    fn build_tracks_clips_match_patterns() {
        let song = make_test_song();
        for track in &song.tracks {
            assert_eq!(track.clips.len(), 2);
        }
    }

    #[test]
    fn build_tracks_cell_data_matches() {
        let song = make_test_song();

        let clip0 = song.tracks[0].clips[0].pattern().unwrap();
        assert_eq!(clip0.channels, 1);
        assert_eq!(clip0.rows, 4);
        assert_eq!(clip0.cell(0, 0).note, crate::pattern::Note::On(48));
        assert_eq!(clip0.cell(0, 0).instrument, 1);

        let clip2 = song.tracks[2].clips[0].pattern().unwrap();
        assert_eq!(clip2.cell(0, 0).note, crate::pattern::Note::On(60));

        let clip1 = song.tracks[1].clips[0].pattern().unwrap();
        assert_eq!(clip1.cell(3, 0).note, crate::pattern::Note::Off);

        let clip3_1 = song.tracks[3].clips[1].pattern().unwrap();
        assert_eq!(clip3_1.cell(0, 0).note, crate::pattern::Note::On(72));
    }

    #[test]
    fn build_tracks_sequences_identical() {
        let song = make_test_song();
        let seq0 = &song.tracks[0].sequence;
        for track in &song.tracks[1..] {
            assert_eq!(&track.sequence, seq0);
        }
    }

    #[test]
    fn build_tracks_sequence_has_correct_times() {
        let song = make_test_song();
        let seq = &song.tracks[0].sequence;
        assert_eq!(seq.len(), 2);
        assert_eq!(seq[0].start, MusicalTime::zero());
        assert_eq!(seq[0].clip_idx, 0);
        assert_eq!(seq[1].start, MusicalTime::from_beats(1));
        assert_eq!(seq[1].clip_idx, 1);
    }

    #[test]
    fn total_time_matches_expected() {
        let song = make_test_song();
        // pat0: 4 rows = 1 beat, pat1: 8 rows = 2 beats → 3 beats
        assert_eq!(song.total_time(), MusicalTime::from_beats(3));
    }

    #[test]
    fn total_time_empty() {
        let song = Song::new("empty");
        assert_eq!(song.total_time(), MusicalTime::zero());
    }

    #[test]
    fn build_tracks_preserves_rows_per_beat() {
        let mut song = Song::with_channels("test", 2);
        let mut pat = Pattern::new(8, 2);
        pat.rows_per_beat = Some(8);
        let patterns = vec![pat];
        let order = vec![OrderEntry::Pattern(0)];
        build_tracks(&mut song, &patterns, &order);

        let clip = song.tracks[0].clips[0].pattern().unwrap();
        assert_eq!(clip.rows_per_beat, Some(8));
    }

    #[test]
    fn build_tracks_preserves_empty_cells() {
        let mut song = Song::with_channels("test", 2);
        let patterns = vec![Pattern::new(4, 2)];
        let order = vec![OrderEntry::Pattern(0)];
        build_tracks(&mut song, &patterns, &order);

        let clip = song.tracks[0].clips[0].pattern().unwrap();
        for row in 0..4 {
            assert!(clip.cell(row, 0).is_empty());
        }
    }

    #[test]
    fn build_tracks_skips_order_skip_entries() {
        let mut song = Song::with_channels("test", 1);
        let patterns = vec![Pattern::new(4, 1)];
        let order = vec![
            OrderEntry::Pattern(0),
            OrderEntry::Skip,
            OrderEntry::Pattern(0),
        ];
        build_tracks(&mut song, &patterns, &order);
        assert_eq!(song.tracks[0].sequence.len(), 2);
    }

    #[test]
    fn build_tracks_stops_at_order_end() {
        let mut song = Song::with_channels("test", 1);
        let patterns = vec![Pattern::new(4, 1)];
        let order = vec![
            OrderEntry::Pattern(0),
            OrderEntry::End,
            OrderEntry::Pattern(0),
        ];
        build_tracks(&mut song, &patterns, &order);
        assert_eq!(song.tracks[0].sequence.len(), 1);
    }
}
