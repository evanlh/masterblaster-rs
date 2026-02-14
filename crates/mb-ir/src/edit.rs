//! Edit commands for mutating song data during playback.

use crate::pattern::Cell;

/// An edit command that mutates song data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Set a single cell in a pattern.
    SetCell {
        pattern: u8,
        row: u16,
        channel: u8,
        cell: Cell,
    },
}
