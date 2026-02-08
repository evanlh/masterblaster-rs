//! Playback engine for masterblaster tracker.
//!
//! Processes the audio graph and event queue to generate audio output.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod channel;
mod event_queue;
mod frame;
mod frequency;
mod mixer;
pub mod scheduler;

pub use channel::ChannelState;
pub use event_queue::EventQueue;
pub use frame::Frame;
pub use frequency::note_to_increment;
pub use mixer::Engine;
pub use scheduler::{schedule_song, ScheduleResult};
