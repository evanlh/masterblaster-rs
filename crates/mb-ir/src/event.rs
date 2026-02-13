//! Event types for the event-driven playback engine.

use crate::effects::Effect;
use crate::graph::NodeId;
use crate::musical_time::MusicalTime;

/// A scheduled event in the song.
#[derive(Clone, Debug)]
pub struct Event {
    /// When the event should fire
    pub time: MusicalTime,
    /// Where the event is routed
    pub target: EventTarget,
    /// What the event does
    pub payload: EventPayload,
}

impl Event {
    /// Create a new event.
    pub fn new(time: MusicalTime, target: EventTarget, payload: EventPayload) -> Self {
        Self {
            time,
            target,
            payload,
        }
    }
}

/// Where an event is routed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventTarget {
    /// Traditional tracker channel (0-255)
    Channel(u8),
    /// Audio graph node (for Buzz machines, synths, effects)
    Node(NodeId),
    /// Global events (tempo, transport)
    Global,
}

/// What an event does.
#[derive(Clone, Debug, PartialEq)]
pub enum EventPayload {
    // === Note events ===
    /// Trigger a note
    NoteOn {
        note: u8,
        velocity: u8,
        instrument: u8,
    },
    /// Release a note
    NoteOff { note: u8 },
    /// Set portamento target (TonePorta + note: don't trigger, just set target)
    PortaTarget { note: u8, instrument: u8 },

    // === Parameter changes ===
    /// Instantly set a parameter value
    ParamChange { param: u16, value: i32 },
    /// Ramp a parameter to a target value over duration ticks
    ParamRamp {
        param: u16,
        target: i32,
        duration: u32,
    },

    // === Transport ===
    /// Set tempo (BPM * 100 for precision, e.g., 12500 = 125.00 BPM)
    SetTempo(u16),
    /// Set speed (ticks per row)
    SetSpeed(u8),

    // === Pattern effects ===
    /// A tracker effect command
    Effect(Effect),
}

impl EventPayload {
    /// Create a note on event with default velocity.
    pub fn note_on(note: u8, instrument: u8) -> Self {
        Self::NoteOn {
            note,
            velocity: 64,
            instrument,
        }
    }
}
