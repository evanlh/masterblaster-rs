//! Priority queue for scheduled events.

use alloc::vec::Vec;
use mb_ir::{Event, MusicalTime};

/// A priority queue of events sorted by timestamp.
#[derive(Clone, Debug, Default)]
pub struct EventQueue {
    events: Vec<Event>,
}

impl EventQueue {
    /// Create a new empty event queue.
    pub fn new() -> Self {
        Self { events: Vec::new() }
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

    /// Pop all events at or before the given timestamp.
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

    /// Clear all events from the queue.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Returns true if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Returns the number of events in the queue.
    pub fn len(&self) -> usize {
        self.events.len()
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
}
