//! Headless controller for masterblaster tracker.
//!
//! Provides a unified API for loading songs, playback, and rendering
//! that both the GUI and CLI can share.

mod wav;

use mb_audio::{AudioOutput, CpalOutput};
use mb_engine::Engine;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

// Re-export common types so callers don't need mb-ir/mb-engine directly.
pub use mb_engine::Frame;
pub use mb_formats::FormatError;
pub use mb_ir::{PlaybackPosition, Song};

pub use wav::{frames_to_wav, write_wav};

/// Headless tracker controller — owns a song and manages playback.
pub struct Controller {
    song: Song,
    playback: Option<PlaybackHandle>,
}

struct PlaybackHandle {
    stop_signal: Arc<AtomicBool>,
    current_tick: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Controller {
    pub fn new() -> Self {
        Self {
            song: Song::with_channels("Untitled", 4),
            playback: None,
        }
    }

    // --- Song management ---

    pub fn song(&self) -> &Song {
        &self.song
    }

    pub fn load_mod(&mut self, data: &[u8]) -> Result<(), FormatError> {
        self.stop();
        self.song = mb_formats::load_mod(data)?;
        Ok(())
    }

    // --- Real-time playback ---

    pub fn play(&mut self) {
        self.stop();

        let song = self.song.clone();
        let stop_signal = Arc::new(AtomicBool::new(false));
        let current_tick = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));

        let stop = stop_signal.clone();
        let tick = current_tick.clone();
        let done = finished.clone();

        let thread = std::thread::spawn(move || {
            audio_thread(song, stop, tick, done);
        });

        self.playback = Some(PlaybackHandle {
            stop_signal,
            current_tick,
            finished,
            thread: Some(thread),
        });
    }

    pub fn stop(&mut self) {
        if let Some(mut pb) = self.playback.take() {
            pb.stop_signal.store(true, Ordering::Relaxed);
            if let Some(handle) = pb.thread.take() {
                let _ = handle.join();
            }
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playback
            .as_ref()
            .is_some_and(|p| !p.finished.load(Ordering::Relaxed))
    }

    pub fn is_finished(&self) -> bool {
        self.playback
            .as_ref()
            .is_some_and(|p| p.finished.load(Ordering::Relaxed))
    }

    pub fn position(&self) -> Option<PlaybackPosition> {
        let pb = self.playback.as_ref()?;
        let tick = pb.current_tick.load(Ordering::Relaxed);
        if pb.finished.load(Ordering::Relaxed) {
            return None;
        }
        mb_ir::tick_to_position(&self.song, tick)
    }

    // --- Offline rendering ---

    pub fn render_frames(&self, sample_rate: u32, max_frames: usize) -> Vec<Frame> {
        let mut engine = Engine::new(self.song.clone(), sample_rate);
        engine.schedule_song();
        engine.play();

        let mut frames = Vec::with_capacity(max_frames);
        while !engine.is_finished() && frames.len() < max_frames {
            frames.push(engine.render_frame());
        }
        frames
    }

    pub fn render_to_wav(&self, sample_rate: u32, max_seconds: u32) -> Vec<u8> {
        let max_frames = (sample_rate * max_seconds) as usize;
        let frames = self.render_frames(sample_rate, max_frames);
        wav::frames_to_wav(&frames, sample_rate)
    }
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

fn audio_thread(
    song: Song,
    stop_signal: Arc<AtomicBool>,
    current_tick: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
) {
    let Ok((mut output, consumer)) = CpalOutput::new() else {
        finished.store(true, Ordering::Relaxed);
        return;
    };

    let sample_rate = output.sample_rate();
    let mut engine = Engine::new(song, sample_rate);
    engine.schedule_song();
    engine.play();

    if output.build_stream(consumer).is_err() {
        finished.store(true, Ordering::Relaxed);
        return;
    }
    let _ = output.start();

    let tick_interval = (sample_rate / 100) as u64;
    let mut frame_count: u64 = 0;

    while !engine.is_finished() && !stop_signal.load(Ordering::Relaxed) {
        output.write_spin(engine.render_frame());
        frame_count += 1;
        if frame_count % tick_interval == 0 {
            current_tick.store(engine.position().tick, Ordering::Relaxed);
        }
    }

    for _ in 0..sample_rate {
        output.write_spin(Frame::silence());
    }

    finished.store(true, Ordering::Relaxed);
}
