//! Audio output trait and error types.

use mb_engine::Frame;

/// Error type for audio operations.
#[derive(Debug)]
pub enum AudioError {
    /// Failed to initialize audio device
    DeviceInit(String),
    /// Failed to create audio stream
    StreamCreate(String),
    /// Playback error
    Playback(String),
    /// No audio device available
    NoDevice,
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::DeviceInit(msg) => write!(f, "Device init error: {}", msg),
            AudioError::StreamCreate(msg) => write!(f, "Stream create error: {}", msg),
            AudioError::Playback(msg) => write!(f, "Playback error: {}", msg),
            AudioError::NoDevice => write!(f, "No audio device available"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Trait for audio output backends.
pub trait AudioOutput {
    /// Get the sample rate.
    fn sample_rate(&self) -> u32;

    /// Write frames to the output (blocking — parks until all frames are written).
    fn write(&mut self, frames: &[Frame]);

    /// Start playback.
    fn start(&mut self) -> Result<(), AudioError>;

    /// Stop playback.
    fn stop(&mut self) -> Result<(), AudioError>;
}
