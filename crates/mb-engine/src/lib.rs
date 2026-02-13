//! Playback engine for masterblaster tracker.
//!
//! Processes the audio graph and event queue to generate audio output.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod channel;
mod event_queue;
mod frame;
mod frequency;
mod graph_state;
pub mod machine;
pub mod machines;
mod mixer;
pub mod scheduler;

pub use channel::ChannelState;
pub use event_queue::EventQueue;
pub use frame::{Frame, WideFrame};
pub use frequency::{note_to_increment, note_to_period, period_to_increment, clamp_period, PERIOD_MIN, PERIOD_MAX};
pub use mixer::Engine;
pub use scheduler::{schedule_song, ScheduleResult};
