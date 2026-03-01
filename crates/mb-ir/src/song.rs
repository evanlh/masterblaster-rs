//! Song structure and sequencing types.

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
    /// Graph: Tracker→AmigaFilter→Master
    pub fn with_channels(title: &str, num_channels: u8) -> Self {
        use crate::graph::{NodeType, Parameter};

        let mut song = Self::new(title);

        // Insert Amiga filter between tracker and master
        let filter_id = song
            .graph
            .add_node(NodeType::BuzzMachine { machine_name: alloc::string::String::from("Amiga Filter") });
        song.graph.node_mut(filter_id).unwrap().parameters.push(
            Parameter::new(0, "Cutoff", 1000, 22050, 4410),
        );
        song.graph.connect(filter_id, 0); // filter → master

        // Single Tracker machine node for all channels
        let tracker_id = song
            .graph
            .add_node(NodeType::BuzzMachine { machine_name: alloc::string::String::from("Tracker") });
        song.graph.connect(tracker_id, filter_id); // tracker → filter

        for i in 0..num_channels {
            song.channels.push(ChannelSettings {
                // Classic Amiga panning: L R R L pattern
                initial_pan: if i % 4 == 0 || i % 4 == 3 { -64 } else { 64 },
                initial_vol: 64,
                muted: false,
            });
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
///
/// Each track represents a machine (or standalone automation lane).
/// Multi-channel tracker machines have one Track with multi-column patterns.
#[derive(Clone, Debug)]
pub struct Track {
    /// Parent machine node in graph (e.g. AmigaFilter for MOD, tracker machine for BMX).
    /// `None` = standalone/automation track.
    pub machine_node: Option<NodeId>,
    /// First TrackerChannel index this track drives.
    pub base_channel: u8,
    /// Number of channels (= pattern column count).
    pub num_channels: u8,
    /// Pool of clips owned by this track
    pub clips: Vec<Clip>,
    /// Playback order (which clip to play when)
    pub sequence: Vec<SeqEntry>,
}

impl Track {
    /// Create a new track with the given channel mapping.
    pub fn new(machine_node: Option<NodeId>, base_channel: u8, num_channels: u8) -> Self {
        Self {
            machine_node,
            base_channel,
            num_channels,
            clips: Vec::new(),
            sequence: Vec::new(),
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

/// Find the Tracker machine node in the graph.
pub fn find_tracker_node(graph: &AudioGraph) -> Option<NodeId> {
    graph.nodes.iter().enumerate()
        .find(|(_, n)| matches!(&n.node_type, NodeType::BuzzMachine { machine_name } if machine_name == "Tracker"))
        .map(|(i, _)| i as NodeId)
}

/// Find the first BuzzMachine node in the graph (e.g. AmigaFilter for MOD).
pub fn find_machine_node(graph: &AudioGraph) -> Option<NodeId> {
    graph.nodes.iter().enumerate()
        .find(|(_, n)| matches!(&n.node_type, NodeType::BuzzMachine { .. }))
        .map(|(i, _)| i as NodeId)
}

/// Build a single track from multi-channel patterns and an order list.
///
/// Creates one Track with the original multi-channel patterns cloned directly
/// (no column extraction). `base_channel = 0`, `num_channels = song.channels.len()`.
pub fn build_tracks(
    song: &mut Song,
    patterns: &[Pattern],
    order: &[OrderEntry],
) {
    let num_channels = song.channels.len() as u8;
    if num_channels == 0 {
        return;
    }

    let machine_node = find_tracker_node(&song.graph);
    let mut track = Track::new(machine_node, 0, num_channels);

    for pattern in patterns {
        track.clips.push(Clip::Pattern(pattern.clone()));
    }

    track.sequence = build_sequence_from_order(order, patterns, song.rows_per_beat);
    song.tracks = alloc::vec![track];
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
    fn build_tracks_creates_one_track() {
        let song = make_test_song();
        assert_eq!(song.tracks.len(), 1);
        assert_eq!(song.tracks[0].num_channels, 4);
        assert_eq!(song.tracks[0].base_channel, 0);
    }

    #[test]
    fn build_tracks_clips_match_patterns() {
        let song = make_test_song();
        assert_eq!(song.tracks[0].clips.len(), 2);
    }

    #[test]
    fn build_tracks_cell_data_matches() {
        let song = make_test_song();
        let track = &song.tracks[0];

        let clip0 = track.clips[0].pattern().unwrap();
        assert_eq!(clip0.channels, 4);
        assert_eq!(clip0.rows, 4);
        assert_eq!(clip0.cell(0, 0).note, crate::pattern::Note::On(48));
        assert_eq!(clip0.cell(0, 0).instrument, 1);
        assert_eq!(clip0.cell(0, 2).note, crate::pattern::Note::On(60));
        assert_eq!(clip0.cell(3, 1).note, crate::pattern::Note::Off);

        let clip1 = track.clips[1].pattern().unwrap();
        assert_eq!(clip1.cell(0, 3).note, crate::pattern::Note::On(72));
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
        build_tracks(&mut song, &[pat], &[OrderEntry::Pattern(0)]);

        let clip = song.tracks[0].clips[0].pattern().unwrap();
        assert_eq!(clip.rows_per_beat, Some(8));
    }

    #[test]
    fn build_tracks_preserves_empty_cells() {
        let mut song = Song::with_channels("test", 2);
        build_tracks(&mut song, &[Pattern::new(4, 2)], &[OrderEntry::Pattern(0)]);

        let clip = song.tracks[0].clips[0].pattern().unwrap();
        for row in 0..4 {
            assert!(clip.cell(row, 0).is_empty());
            assert!(clip.cell(row, 1).is_empty());
        }
    }

    #[test]
    fn build_tracks_skips_order_skip_entries() {
        let mut song = Song::with_channels("test", 1);
        build_tracks(&mut song, &[Pattern::new(4, 1)], &[
            OrderEntry::Pattern(0), OrderEntry::Skip, OrderEntry::Pattern(0),
        ]);
        assert_eq!(song.tracks[0].sequence.len(), 2);
    }

    #[test]
    fn build_tracks_stops_at_order_end() {
        let mut song = Song::with_channels("test", 1);
        build_tracks(&mut song, &[Pattern::new(4, 1)], &[
            OrderEntry::Pattern(0), OrderEntry::End, OrderEntry::Pattern(0),
        ]);
        assert_eq!(song.tracks[0].sequence.len(), 1);
    }

    #[test]
    fn build_tracks_machine_node_points_to_tracker() {
        let song = make_test_song();
        let machine = song.tracks[0].machine_node;
        assert!(machine.is_some());
        let node = song.graph.node(machine.unwrap()).unwrap();
        assert!(matches!(&node.node_type, NodeType::BuzzMachine { machine_name } if machine_name == "Tracker"));
    }
}
