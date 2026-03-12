//! EventSource trait for lazy event generation.

use alloc::vec::Vec;
use mb_ir::{Event, MusicalTime, Song};

/// A lazy source of events that generates them on demand as playback advances.
pub trait EventSource {
    /// Drain all events up to (and including) `time` into `out`.
    /// Returns the number of events added.
    fn drain_until(&mut self, time: MusicalTime, song: &Song, out: &mut Vec<Event>) -> usize;

    /// Seek to a new position, resetting internal cursor state.
    fn seek(&mut self, time: MusicalTime, song: &Song);

    /// Peek at the time of the next event without consuming it.
    /// Returns `None` if the source is exhausted.
    fn peek_time(&self) -> Option<MusicalTime>;
}
