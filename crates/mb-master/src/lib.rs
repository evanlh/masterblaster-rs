//! Headless controller for masterblaster tracker.
//!
//! Provides a unified API for loading songs, playback, and rendering
//! that both the GUI and CLI can share.

use mb_audio::{AudioOutput, CpalOutput};
use mb_engine::Engine;
use mb_ir::BLOCK_SIZE;
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

// Re-export common types so callers don't need mb-ir/mb-engine directly.
pub use mb_formats::{FormatError, frames_to_wav, load_wav, write_wav};
pub use mb_ir::{pack_time, unpack_time, Edit, PlaybackPosition, Song, TrackPlaybackPosition, time_to_track_position};

/// Ring buffer capacity for edit commands sent to the audio thread.
const EDIT_RING_CAPACITY: usize = 256;

// ---------------------------------------------------------------------------
// Allocation guards — no-ops without the `alloc_check` feature.
// ---------------------------------------------------------------------------

/// Wrap `f` in assert_no_alloc (aborts on heap allocation).
#[cfg(feature = "alloc_check")]
fn alloc_guard<R>(f: impl FnOnce() -> R) -> R {
    assert_no_alloc::assert_no_alloc(f)
}
#[cfg(not(feature = "alloc_check"))]
#[inline(always)]
fn alloc_guard<R>(f: impl FnOnce() -> R) -> R { f() }

/// Temporarily permit allocations inside an `alloc_guard` block.
#[cfg(feature = "alloc_check")]
fn alloc_permit<R>(f: impl FnOnce() -> R) -> R {
    assert_no_alloc::permit_alloc(f)
}
#[cfg(not(feature = "alloc_check"))]
#[inline(always)]
fn alloc_permit<R>(f: impl FnOnce() -> R) -> R { f() }

/// Headless tracker controller — owns a song and manages playback.
pub struct Controller {
    song: Song,
    playback: Option<PlaybackHandle>,
}

struct PlaybackHandle {
    stop_signal: Arc<AtomicBool>,
    /// Packed MusicalTime: (beat as u32) << 32 | sub_beat
    current_time: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    edit_producer: ringbuf::HeapProd<Edit>,
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

    pub fn set_song(&mut self, song: Song) {
        self.stop();
        self.song = song;
    }

    pub fn load_mod(&mut self, data: &[u8]) -> Result<(), FormatError> {
        self.stop();
        self.song = mb_formats::load_mod(data)?;
        Ok(())
    }

    pub fn load_bmx(&mut self, data: &[u8]) -> Result<(), FormatError> {
        self.stop();
        self.song = mb_formats::load_bmx(data)?;
        Ok(())
    }

    /// Create a new empty song with default settings.
    pub fn new_song(&mut self, channels: u8) {
        self.stop();
        let mut song = Song::with_channels("Untitled", channels);
        let patterns = vec![mb_ir::Pattern::new(64, channels)];
        let order = vec![mb_ir::OrderEntry::Pattern(0)];
        mb_ir::build_tracks(&mut song, &patterns, &order);
        self.song = song;
    }

    /// Load a WAV file as a sample and add it to the song.
    /// Returns the 1-based instrument number on success.
    pub fn load_wav_sample(&mut self, data: &[u8], name: &str) -> Result<u8, FormatError> {
        let sample = mb_formats::load_wav(data, name)?;
        let sample_idx = self.song.samples.len() as u8;
        self.song.samples.push(sample);

        let mut inst = mb_ir::Instrument::new(name);
        inst.set_single_sample(sample_idx);
        self.song.instruments.push(inst);

        Ok(self.song.instruments.len() as u8) // 1-based
    }

    /// Add a new empty clip to all tracks in the given group.
    /// Returns the clip index (same across all tracks in the group).
    pub fn add_clip(&mut self, group: Option<u16>, rows: u16) -> u16 {
        let clip_idx = group_clip_count(&self.song, group);
        for track in &mut self.song.tracks {
            if track.group == group {
                track.clips.push(mb_ir::Clip::Pattern(mb_ir::Pattern::new(rows, 1)));
            }
        }
        clip_idx
    }

    /// Add a sequence entry to all tracks in the given group.
    pub fn add_seq_entry(&mut self, group: Option<u16>, clip_idx: u16) {
        let start = group_end_time(&self.song, group);
        let entry = mb_ir::SeqEntry { start, clip_idx };
        for track in &mut self.song.tracks {
            if track.group == group {
                track.sequence.push(entry);
            }
        }
    }

    /// Remove the last sequence entry from all tracks in the given group.
    pub fn remove_last_seq_entry(&mut self, group: Option<u16>) {
        for track in &mut self.song.tracks {
            if track.group == group {
                track.sequence.pop();
            }
        }
    }

    // --- Edit dispatch ---

    /// Apply an edit to the local song and push it to the audio thread if playing.
    pub fn apply_edit(&mut self, edit: Edit) {
        apply_edit_to_song(&mut self.song, &edit);
        if let Some(pb) = &mut self.playback {
            let _ = pb.edit_producer.try_push(edit);
        }
    }

    // --- Real-time playback ---

    pub fn play(&mut self) {
        self.play_song(self.song.clone());
    }

    pub fn play_pattern(&mut self, clip_idx: usize) {
        self.play_song(self.single_clip_song(clip_idx as u16));
    }

    fn play_song(&mut self, song: Song) {
        self.stop();

        let stop_signal = Arc::new(AtomicBool::new(false));
        let current_time = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));

        let rb = HeapRb::<Edit>::new(EDIT_RING_CAPACITY);
        let (edit_producer, edit_consumer) = rb.split();

        let stop = stop_signal.clone();
        let time = current_time.clone();
        let done = finished.clone();

        let thread = std::thread::spawn(move || {
            audio_thread(song, stop, time, done, edit_consumer);
        });

        self.playback = Some(PlaybackHandle {
            stop_signal,
            current_time,
            finished,
            thread: Some(thread),
            edit_producer,
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

    /// Get the current playback position in per-track coordinates.
    pub fn track_position(&self, group: Option<u16>) -> Option<TrackPlaybackPosition> {
        let pb = self.playback.as_ref()?;
        if pb.finished.load(Ordering::Relaxed) {
            return None;
        }
        let packed = pb.current_time.load(Ordering::Relaxed);
        let time = unpack_time(packed);
        time_to_track_position(&self.song, time, group)
    }

    // --- Offline rendering ---

    pub fn render_frames(&self, sample_rate: u32, max_frames: usize) -> Vec<[f32; 2]> {
        render_song_frames(self.song.clone(), sample_rate, max_frames)
    }

    pub fn render_to_wav(&self, sample_rate: u32, max_seconds: u32) -> Vec<u8> {
        render_song_to_wav(self.song.clone(), sample_rate, max_seconds)
    }

    pub fn render_pattern_to_wav(&self, clip_idx: usize, sample_rate: u32, max_seconds: u32) -> Vec<u8> {
        render_song_to_wav(self.single_clip_song(clip_idx as u16), sample_rate, max_seconds)
    }

    // --- Helpers ---

    /// Build a song that plays only the given clip.
    fn single_clip_song(&self, clip_idx: u16) -> Song {
        let mut song = self.song.clone();
        rebuild_track_sequences(&mut song, clip_idx);
        song
    }
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

/// Apply an edit directly to song data (no event queue update).
fn apply_edit_to_song(song: &mut Song, edit: &Edit) {
    match edit {
        Edit::SetCell { track, clip, row, column, cell } => {
            let Some(t) = song.tracks.get_mut(*track as usize) else { return };
            let Some(c) = t.clips.get_mut(*clip as usize) else { return };
            let Some(pat) = c.pattern_mut() else { return };
            if *row < pat.rows && *column < pat.channels {
                *pat.cell_mut(*row, *column) = *cell;
            }
        }
    }
}

/// Rebuild track sequences to play only a single clip (by clip index).
fn rebuild_track_sequences(song: &mut Song, clip_idx: u16) {
    use mb_ir::SeqEntry;
    let entry = SeqEntry { start: mb_ir::MusicalTime::zero(), clip_idx };
    for track in &mut song.tracks {
        track.sequence = if (clip_idx as usize) < track.clips.len() {
            vec![entry]
        } else {
            Vec::new()
        };
    }
}

fn render_song_frames(song: Song, sample_rate: u32, max_frames: usize) -> Vec<[f32; 2]> {
    let mut engine = Engine::new(song, sample_rate);
    engine.schedule_song();
    engine.play();

    let mut frames = Vec::with_capacity(max_frames);
    while !engine.is_finished() && frames.len() < max_frames {
        frames.push(engine.render_frame());
    }
    frames
}

fn render_song_to_wav(song: Song, sample_rate: u32, max_seconds: u32) -> Vec<u8> {
    let max_frames = (sample_rate * max_seconds) as usize;
    let frames = render_song_frames(song, sample_rate, max_frames);
    frames_to_wav(&frames, sample_rate)
}

fn audio_thread(
    song: Song,
    stop_signal: Arc<AtomicBool>,
    current_time: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    mut edit_consumer: ringbuf::HeapCons<Edit>,
) {
    let Ok((mut output, consumer)) = CpalOutput::new() else {
        finished.store(true, Ordering::Relaxed);
        return;
    };

    let sample_rate = output.sample_rate();
    let mut engine = Engine::new(song, sample_rate);
    engine.schedule_song();

    alloc_guard(|| {
        engine.play();

        alloc_permit(|| {
            if output.build_stream(consumer, std::thread::current()).is_err() {
                return;
            }
            let _ = output.start();
        });

        run_audio_loop(
            &mut engine, &mut output, &stop_signal, &current_time,
            &mut edit_consumer, sample_rate,
        );
    });

    finished.store(true, Ordering::Relaxed);
}

/// Main audio render loop. Must be called inside `alloc_guard`.
fn run_audio_loop(
    engine: &mut Engine,
    output: &mut CpalOutput,
    stop_signal: &AtomicBool,
    current_time: &AtomicU64,
    edit_consumer: &mut ringbuf::HeapCons<Edit>,
    sample_rate: u32,
) {
    let report_interval = (sample_rate / 100) as u64;
    let mut frame_count: u64 = 0;
    let mut edit_buf: Vec<Edit> = alloc_permit(Vec::new);
    let mut batch = [[0.0f32; 2]; BLOCK_SIZE];
    let mut interleaved = [0.0f32; BLOCK_SIZE * 2];

    while !engine.is_finished() && !stop_signal.load(Ordering::Relaxed) {
        alloc_permit(|| drain_edits(edit_consumer, &mut edit_buf));
        if !edit_buf.is_empty() {
            engine.apply_edits(&edit_buf);
            edit_buf.clear();
        }

        let n = frames_until_report(frame_count, report_interval, BLOCK_SIZE);
        engine.render_frames_into(&mut batch[..n]);

        // Interleave for output
        for i in 0..n {
            interleaved[i * 2] = batch[i][0];
            interleaved[i * 2 + 1] = batch[i][1];
        }
        output.write(&interleaved[..n * 2]);

        frame_count += n as u64;
        if frame_count.is_multiple_of(report_interval) {
            current_time.store(pack_time(engine.position()), Ordering::Relaxed);
        }
    }

    let silence = [0.0f32; BLOCK_SIZE * 2];
    let tail_frames = sample_rate as usize;
    let mut written = 0;
    while written < tail_frames {
        let n = (tail_frames - written).min(BLOCK_SIZE);
        output.write(&silence[..n * 2]);
        written += n;
    }
}

/// Drain all available edits from the consumer into the buffer.
fn drain_edits(consumer: &mut ringbuf::HeapCons<Edit>, buf: &mut Vec<Edit>) {
    while let Some(edit) = consumer.try_pop() {
        buf.push(edit);
    }
}

/// Frames to render before the next position report, clamped to batch_size.
fn frames_until_report(frame_count: u64, interval: u64, batch_size: usize) -> usize {
    let remaining = interval - (frame_count % interval);
    (remaining as usize).min(batch_size)
}

/// Number of clips in the first track of the given group.
fn group_clip_count(song: &Song, group: Option<u16>) -> u16 {
    song.tracks.iter()
        .find(|t| t.group == group)
        .map(|t| t.clips.len() as u16)
        .unwrap_or(0)
}

/// End time of the group's sequence (after the last clip finishes).
fn group_end_time(song: &Song, group: Option<u16>) -> mb_ir::MusicalTime {
    let rpb = song.rows_per_beat as u32;
    song.tracks.iter()
        .find(|t| t.group == group)
        .and_then(|t| {
            let last = t.sequence.last()?;
            let clip = t.clips.get(last.clip_idx as usize)?;
            let pat = clip.pattern()?;
            let pat_rpb = pat.rows_per_beat.map_or(rpb, |r| r as u32);
            Some(last.start.add_rows(pat.rows as u32, pat_rpb))
        })
        .unwrap_or(mb_ir::MusicalTime::zero())
}
