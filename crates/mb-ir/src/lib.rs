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
mod audio_buffer;
mod audio_traits;
mod edit;
mod effects;
mod event;
mod graph;
mod instrument;
mod mod_envelope;
mod modulator;
mod pattern;
mod sample;
pub mod song;
mod musical_time;

pub use analysis::{analyze_pattern, time_to_track_position, PatternFeatures, PlaybackPosition, TrackPlaybackPosition};
pub use audio_buffer::{AudioBuffer, BLOCK_SIZE, MAX_CHANNELS};
pub use audio_traits::{AudioSource, AudioStream, ChannelConfig};
pub use edit::Edit;
pub use effects::{Effect, VolumeCommand};
pub use event::{Event, EventPayload, EventTarget};
pub use graph::{AudioGraph, Connection, Node, NodeId, NodeType, Parameter};
pub use instrument::{DuplicateCheck, Envelope, EnvelopePoint, Instrument, NewNoteAction};
pub use mod_envelope::{interpolate, CurveKind, LoopRange, ModBreakPoint, ModEnvelope};
pub use modulator::{
    adsr_envelope, arpeggio_envelope, note_cut_envelope, porta_envelope, retrigger_envelope,
    sub_beats_per_tick, tone_porta_envelope, add_mode_sine_envelope,
    volume_slide_envelope, ChannelParam, GlobalParam, ModMode, ModTarget, Modulator,
};
pub use musical_time::{unpack_time, pack_time, MusicalTime, SUB_BEAT_UNIT};
pub use pattern::{Cell, Note, Pattern};
pub use sample::{AutoVibrato, LoopType, Sample, SampleData};
pub use song::{build_tracks, ChannelSettings, Clip, OrderEntry, SeqEntry, Song, Track};
