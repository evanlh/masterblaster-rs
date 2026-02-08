//! Core IR types for masterblaster tracker.
//!
//! This crate defines the intermediate representation used throughout
//! the tracker. All format parsers emit IR, and the playback engine
//! consumes IR.
//!
//! Designed to be `no_std` compatible with the `alloc` crate.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod analysis;
mod effects;
mod event;
mod graph;
mod instrument;
mod pattern;
mod sample;
mod song;
mod timestamp;

pub use analysis::{analyze, SongFeatures};
pub use effects::{Effect, VolumeCommand};
pub use event::{Event, EventPayload, EventTarget};
pub use graph::{AudioGraph, Connection, Node, NodeId, NodeType, Parameter};
pub use instrument::{DuplicateCheck, Envelope, EnvelopePoint, Instrument, NewNoteAction};
pub use pattern::{Cell, Note, Pattern};
pub use sample::{AutoVibrato, LoopType, Sample, SampleData};
pub use song::{ChannelSettings, OrderEntry, Song, Track, TrackEntry};
pub use timestamp::Timestamp;
