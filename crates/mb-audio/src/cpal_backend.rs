//! CPAL-based audio output backend.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};
use mb_engine::Frame;
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::traits::{AudioError, AudioOutput};

/// CPAL-based audio output.
pub struct CpalOutput {
    device: Device,
    config: StreamConfig,
    stream: Option<Stream>,
    producer: HeapProd<Frame>,
    running: Arc<AtomicBool>,
}

impl CpalOutput {
    /// Create a new CPAL output with default device.
    pub fn new() -> Result<(Self, HeapCons<Frame>), AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoDevice)?;

        let config = device
            .default_output_config()
            .map_err(|e| AudioError::DeviceInit(e.to_string()))?;

        let config: StreamConfig = config.into();

        // Create ring buffer for audio data (about 100ms buffer)
        let buffer_size = (config.sample_rate.0 as usize / 10) * 2;
        let rb = HeapRb::<Frame>::new(buffer_size);
        let (producer, consumer) = rb.split();

        let output = Self {
            device,
            config,
            stream: None,
            producer,
            running: Arc::new(AtomicBool::new(false)),
        };

        Ok((output, consumer))
    }

    /// Build and start the audio stream.
    pub fn build_stream(&mut self, mut consumer: HeapCons<Frame>) -> Result<(), AudioError> {
        let running = self.running.clone();

        let stream = self.device
            .build_output_stream(
                &self.config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if !running.load(Ordering::Relaxed) {
                        for sample in data.iter_mut() {
                            *sample = 0.0;
                        }
                        return;
                    }

                    for chunk in data.chunks_mut(2) {
                        if let Some(frame) = consumer.try_pop() {
                            chunk[0] = frame.left as f32 / 32768.0;
                            chunk[1] = frame.right as f32 / 32768.0;
                        } else {
                            chunk[0] = 0.0;
                            chunk[1] = 0.0;
                        }
                    }
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            )
            .map_err(|e| AudioError::StreamCreate(e.to_string()))?;

        stream.play().map_err(|e| AudioError::Playback(e.to_string()))?;
        self.stream = Some(stream);

        Ok(())
    }
}

impl AudioOutput for CpalOutput {
    fn sample_rate(&self) -> u32 {
        self.config.sample_rate.0
    }

    fn write(&mut self, frames: &[Frame]) -> Result<(), AudioError> {
        for frame in frames {
            // Non-blocking push; drop frames if buffer is full
            let _ = self.producer.try_push(*frame);
        }
        Ok(())
    }

    fn start(&mut self) -> Result<(), AudioError> {
        self.running.store(true, Ordering::Relaxed);
        if let Some(ref stream) = self.stream {
            stream.play().map_err(|e| AudioError::Playback(e.to_string()))?;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        self.running.store(false, Ordering::Relaxed);
        if let Some(ref stream) = self.stream {
            stream.pause().map_err(|e| AudioError::Playback(e.to_string()))?;
        }
        Ok(())
    }
}
