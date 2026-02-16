//! Priority queue for scheduled events.

use alloc::vec::Vec;
use mb_ir::{Event, MusicalTime};

/// A priority queue of events sorted by timestamp.
///
/// During playback, events are consumed via a cursor that advances forward
/// without removing elements — making the realtime drain path allocation-free.
#[derive(Clone, Debug, Default)]
pub struct EventQueue {
    events: Vec<Event>,
    /// Next event index to process (advances during playback).
    cursor: usize,
}

impl EventQueue {
    /// Create a new empty event queue.
    pub fn new() -> Self {
        Self { events: Vec::new(), cursor: 0 }
    }

    /// Push an event into the queue.
    pub fn push(&mut self, event: Event) {
        // Find insertion point to maintain sorted order
        let pos = self
            .events
            .binary_search_by(|e| e.time.cmp(&event.time))
            .unwrap_or_else(|pos| pos);
        self.events.insert(pos, event);
    }

    /// Peek at the next event without removing it.
    pub fn peek(&self) -> Option<&Event> {
        self.events.first()
    }

    /// Pop the next event from the queue.
    pub fn pop(&mut self) -> Option<Event> {
        if self.events.is_empty() {
            None
        } else {
            Some(self.events.remove(0))
        }
    }

    /// Return the index range of events at or before `time` (cursor-based, zero allocation).
    ///
    /// Advances the internal cursor past all consumed events. The returned
    /// range can be used to index `self.events` directly.
    pub fn drain_until(&mut self, time: MusicalTime) -> core::ops::Range<usize> {
        let start = self.cursor;
        while self.cursor < self.events.len() {
            if self.events[self.cursor].time <= time {
                self.cursor += 1;
            } else {
                break;
            }
        }
        start..self.cursor
    }

    /// Get an event by index (for use with `drain_until` ranges).
    pub fn get(&self, index: usize) -> Option<&Event> {
        self.events.get(index)
    }

    /// Pop all events at or before the given timestamp (allocates — setup phase only).
    pub fn pop_until(&mut self, time: MusicalTime) -> Vec<Event> {
        let mut result = Vec::new();
        while let Some(event) = self.events.first() {
            if event.time <= time {
                result.push(self.events.remove(0));
            } else {
                break;
            }
        }
        result
    }

    /// Reset cursor to the beginning (called after scheduling).
    pub fn reset_cursor(&mut self) {
        self.cursor = 0;
    }

    /// Clear all events and reset cursor.
    pub fn clear(&mut self) {
        self.events.clear();
        self.cursor = 0;
    }

    /// Returns true if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Returns the number of events in the queue.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Retain only events matching the predicate, removing the rest.
    pub fn retain<F: FnMut(&Event) -> bool>(&mut self, f: F) {
        self.events.retain(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::{EventPayload, EventTarget};

    #[test]
    fn event_ordering() {
        let mut queue = EventQueue::new();

        queue.push(Event::new(
            MusicalTime::from_beats(10),
            EventTarget::Global,
            EventPayload::SetTempo(12500),
        ));
        queue.push(Event::new(
            MusicalTime::from_beats(5),
            EventTarget::Global,
            EventPayload::SetSpeed(6),
        ));
        queue.push(Event::new(
            MusicalTime::from_beats(15),
            EventTarget::Global,
            EventPayload::SetTempo(14000),
        ));

        assert_eq!(queue.pop().unwrap().time.beat, 5);
        assert_eq!(queue.pop().unwrap().time.beat, 10);
        assert_eq!(queue.pop().unwrap().time.beat, 15);
    }

    #[test]
    fn drain_until_returns_range() {
        let mut queue = EventQueue::new();
        queue.push(Event::new(MusicalTime::from_beats(5), EventTarget::Global, EventPayload::SetSpeed(6)));
        queue.push(Event::new(MusicalTime::from_beats(10), EventTarget::Global, EventPayload::SetTempo(12500)));
        queue.push(Event::new(MusicalTime::from_beats(15), EventTarget::Global, EventPayload::SetTempo(14000)));

        let range = queue.drain_until(MusicalTime::from_beats(12));
        assert_eq!(range, 0..2);
        assert_eq!(queue.get(0).unwrap().time.beat, 5);
        assert_eq!(queue.get(1).unwrap().time.beat, 10);
    }

    #[test]
    fn drain_until_advances_cursor() {
        let mut queue = EventQueue::new();
        queue.push(Event::new(MusicalTime::from_beats(5), EventTarget::Global, EventPayload::SetSpeed(6)));
        queue.push(Event::new(MusicalTime::from_beats(10), EventTarget::Global, EventPayload::SetTempo(12500)));

        let r1 = queue.drain_until(MusicalTime::from_beats(7));
        assert_eq!(r1, 0..1);

        let r2 = queue.drain_until(MusicalTime::from_beats(15));
        assert_eq!(r2, 1..2);
    }

    #[test]
    fn reset_cursor_allows_replay() {
        let mut queue = EventQueue::new();
        queue.push(Event::new(MusicalTime::from_beats(1), EventTarget::Global, EventPayload::SetSpeed(6)));

        let r1 = queue.drain_until(MusicalTime::from_beats(5));
        assert_eq!(r1.len(), 1);

        queue.reset_cursor();
        let r2 = queue.drain_until(MusicalTime::from_beats(5));
        assert_eq!(r2.len(), 1);
    }
}
