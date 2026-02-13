//! Song structure and sequencing types.

use alloc::vec::Vec;
use arrayvec::ArrayString;

use crate::graph::{AudioGraph, NodeId};
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
    /// All patterns in the song
    pub patterns: Vec<Pattern>,
    /// Pattern play order
    pub order: Vec<OrderEntry>,
    /// Instruments
    pub instruments: Vec<Instrument>,
    /// Samples
    pub samples: Vec<Sample>,
    /// Per-channel settings
    pub channels: Vec<ChannelSettings>,
    /// Audio routing graph
    pub graph: AudioGraph,
    /// Tracks (for timeline-based sequencing)
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
            patterns: Vec::new(),
            order: Vec::new(),
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

    /// Get the total length of the song as a `MusicalTime`.
    pub fn total_time(&self) -> MusicalTime {
        let rpb = self.rows_per_beat as u32;
        let mut time = MusicalTime::zero();
        for entry in &self.order {
            if let OrderEntry::Pattern(idx) = entry {
                if let Some(pattern) = self.patterns.get(*idx as usize) {
                    let pat_rpb = pattern.rows_per_beat.map_or(rpb, |r| r as u32);
                    time = time.add_rows(pattern.rows as u32, pat_rpb);
                }
            }
        }
        time
    }

    /// Add a pattern and return its index.
    pub fn add_pattern(&mut self, pattern: Pattern) -> u8 {
        let idx = self.patterns.len() as u8;
        self.patterns.push(pattern);
        idx
    }

    /// Add an order entry.
    pub fn add_order(&mut self, entry: OrderEntry) {
        self.order.push(entry);
    }
}

/// An entry in the pattern order list.
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

/// A track in the timeline (for DAW-style arrangement).
#[derive(Clone, Debug)]
pub struct Track {
    /// Which graph node this track controls
    pub target: NodeId,
    /// Name of the track
    pub name: ArrayString<32>,
    /// Sequenced entries (patterns, clips)
    pub entries: Vec<TrackEntry>,
}

impl Track {
    /// Create a new track targeting a node.
    pub fn new(target: NodeId, name: &str) -> Self {
        let mut track_name = ArrayString::new();
        let _ = track_name.try_push_str(name);
        Self {
            target,
            name: track_name,
            entries: Vec::new(),
        }
    }
}

/// An entry in a track's timeline.
#[derive(Clone, Debug)]
pub enum TrackEntry {
    /// A pattern placed at a specific time
    Pattern {
        start: MusicalTime,
        pattern_id: u16,
    },
    /// A MIDI clip (future)
    MidiClip {
        start: MusicalTime,
        clip_id: u16,
    },
    /// An audio clip (future)
    AudioClip {
        start: MusicalTime,
        clip_id: u16,
    },
}
