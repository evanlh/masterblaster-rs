//! Playback engine for masterblaster tracker.
//!
//! Processes the audio graph and event queue to generate audio output.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod channel;
pub mod envelope_state;
mod event_queue;
mod frequency;
mod graph_state;
pub mod machine;
pub mod machines;
mod mixer;
pub mod scheduler;
pub mod voice;
pub mod voice_pool;

pub use channel::ChannelState;
pub use envelope_state::EnvelopeState;
pub use event_queue::EventQueue;
pub use frequency::{note_to_increment, note_to_period, period_to_increment, clamp_period, PERIOD_MIN, PERIOD_MAX};
pub use mixer::Engine;
pub use scheduler::{schedule_cell, schedule_song, time_for_track_clip_row, ScheduleResult};
pub use voice::Voice;
pub use voice_pool::{VoiceId, VoicePool};
