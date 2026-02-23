//! Audio source and stream traits for the audio graph.

use crate::audio_buffer::AudioBuffer;

/// Read-only access to sample data at a given position.
pub trait AudioSource {
    /// Number of channels in the source.
    fn channels(&self) -> u16;

    /// Number of frames in the source.
    fn frames(&self) -> usize;

    /// Read a sample as i16 at the given channel and frame.
    fn read_i16(&self, ch: u16, frame: usize) -> i16;

    /// Read a sample as f32 at the given channel and frame.
    fn read_f32(&self, ch: u16, frame: usize) -> f32 {
        self.read_i16(ch, frame) as f32 / 32768.0
    }
}

/// Channel configuration for an audio processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelConfig {
    pub inputs: u16,
    pub outputs: u16,
}

/// A renderable audio processor (machine, effect, generator).
pub trait AudioStream: Send {
    /// Describe the channel layout.
    fn channel_config(&self) -> ChannelConfig;

    /// Process audio in-place on the buffer.
    fn render(&mut self, output: &mut AudioBuffer);
}

