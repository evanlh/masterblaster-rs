//! Edit commands for mutating song data during playback.

use crate::pattern::Cell;
use crate::song::SeqTermination;

/// Data for placing a sequence entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeqEntryData {
    pub clip_idx: u16,
    pub length: u16,
    pub termination: SeqTermination,
}

/// An edit command that mutates song data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Set a single cell in a track's clip.
    SetCell {
        track: u16,
        clip: u16,
        row: u16,
        column: u8,
        cell: Cell,
    },
    /// Bypass (mute) or unbypass a graph node.
    SetNodeBypass { node: u16, bypassed: bool },
    /// Set or remove a sequence entry at a given beat.
    SetSeqEntry {
        track: u16,
        beat: u32,
        entry: Option<SeqEntryData>,
    },
}
