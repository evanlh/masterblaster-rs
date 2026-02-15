//! Edit commands for mutating song data during playback.

use crate::pattern::Cell;

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
}
