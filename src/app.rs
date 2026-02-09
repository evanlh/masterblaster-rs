//! Application state and audio thread — no GUI dependency.

use mb_audio::{AudioOutput, CpalOutput};
use mb_engine::{Engine, Frame};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

// --- Playback state ---

pub struct PlaybackState {
    pub stop_signal: Arc<AtomicBool>,
    pub current_tick: Arc<AtomicU64>,
    pub finished: Arc<AtomicBool>,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

// --- Center view toggle ---

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CenterView {
    Pattern,
    Graph,
}

// --- App state ---

pub struct TrackerApp {
    pub song: mb_ir::Song,
    pub selected_pattern: usize,
    pub status: String,
    pub playback: Option<PlaybackState>,
    pub center_view: CenterView,
}

impl TrackerApp {
    pub fn new() -> Self {
        Self {
            song: mb_ir::Song::with_channels("Untitled", 4),
            selected_pattern: 0,
            status: String::new(),
            playback: None,
            center_view: CenterView::Pattern,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playback
            .as_ref()
            .is_some_and(|p| !p.finished.load(Ordering::Relaxed))
    }

    pub fn current_tick(&self) -> Option<u64> {
        self.playback
            .as_ref()
            .map(|p| p.current_tick.load(Ordering::Relaxed))
    }

    pub fn playback_position(&self) -> Option<mb_ir::PlaybackPosition> {
        let tick = self.current_tick()?;
        if !self.is_playing() {
            return None;
        }
        mb_ir::tick_to_position(&self.song, tick)
    }

    pub fn load_mod_file(&mut self) {
        let file = rfd::FileDialog::new()
            .add_filter("MOD files", &["mod", "MOD"])
            .pick_file();

        let Some(path) = file else { return };

        self.stop_playback();

        match std::fs::read(&path) {
            Err(e) => self.status = format!("Read error: {}", e),
            Ok(data) => match mb_formats::load_mod(&data) {
                Err(e) => self.status = format!("Parse error: {:?}", e),
                Ok(song) => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    self.status = format!("Loaded {}", name);
                    self.song = song;
                    self.selected_pattern = 0;
                }
            },
        }
    }

    pub fn start_playback(&mut self) {
        self.stop_playback();

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

        self.playback = Some(PlaybackState {
            stop_signal,
            current_tick,
            finished,
            thread: Some(thread),
        });
        self.status = "Playing...".to_string();
    }

    pub fn stop_playback(&mut self) {
        if let Some(mut pb) = self.playback.take() {
            pb.stop_signal.store(true, Ordering::Relaxed);
            if let Some(handle) = pb.thread.take() {
                let _ = handle.join();
            }
            self.status = "Stopped".to_string();
        }
    }
}

fn audio_thread(
    song: mb_ir::Song,
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
