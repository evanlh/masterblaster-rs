//! Audio output backends for masterblaster tracker.

mod cpal_backend;
mod traits;

pub use cpal_backend::CpalOutput;
pub use traits::{AudioError, AudioOutput};
