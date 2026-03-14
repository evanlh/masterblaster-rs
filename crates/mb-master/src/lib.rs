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

    /// Add a new empty clip to the given track.
    /// Returns the clip index.
    pub fn add_clip(&mut self, track_idx: usize, rows: u16) -> u16 {
        let Some(track) = self.song.tracks.get_mut(track_idx) else { return 0 };
        let clip_idx = track.clips.len() as u16;
        let channels = track.num_channels;
        track.clips.push(mb_ir::Clip::Pattern(mb_ir::Pattern::new(rows, channels)));
        clip_idx
    }

    /// Add a sequence entry to the given track.
    pub fn add_seq_entry(&mut self, track_idx: usize, clip_idx: u16) {
        let start = track_end_time(&self.song, track_idx);
        let length = self.song.tracks.get(track_idx)
            .and_then(|t| t.get_pattern_at(clip_idx as usize))
            .map_or(0, |p| p.rows);
        let entry = mb_ir::SeqEntry { start, clip_idx, length, termination: mb_ir::SeqTermination::Natural };
        if let Some(track) = self.song.tracks.get_mut(track_idx) {
            track.sequence.push(entry);
        }
    }

    /// Remove the last sequence entry from the given track.
    pub fn remove_last_seq_entry(&mut self, track_idx: usize) {
        if let Some(track) = self.song.tracks.get_mut(track_idx) {
            track.sequence.pop();
        }
    }

    /// Place a clip at a specific beat in a track's sequence.
    /// Returns the forward and reverse edits, or None if overlap detected.
    pub fn set_seq_entry(&mut self, track_idx: usize, beat: u32, clip_idx: u16) -> Option<(Edit, Edit)> {
        let track = self.song.tracks.get(track_idx)?;
        let length = track.get_pattern_at(clip_idx as usize).map_or(16, |p| p.rows);
        let rpb = self.song.rows_per_beat;
        if would_overlap(track, beat, length, rpb) {
            return None;
        }
        let data = mb_ir::SeqEntryData {
            clip_idx,
            length,
            termination: mb_ir::SeqTermination::Natural,
        };
        // Build reverse: restore whatever was at this beat before
        let old_entry = track.seq_entry_at_beat(beat).map(|e| mb_ir::SeqEntryData {
            clip_idx: e.clip_idx,
            length: e.length,
            termination: e.termination,
        });
        let forward = Edit::SetSeqEntry { track: track_idx as u16, beat, entry: Some(data) };
        let reverse = Edit::SetSeqEntry { track: track_idx as u16, beat, entry: old_entry };
        self.apply_edit(forward.clone());
        Some((forward, reverse))
    }

    /// Remove a sequence entry at a specific beat.
    /// Returns the forward and reverse edits, or None if nothing there.
    pub fn remove_seq_entry(&mut self, track_idx: usize, beat: u32) -> Option<(Edit, Edit)> {
        let track = self.song.tracks.get(track_idx)?;
        let old = track.seq_entry_at_beat(beat)?;
        let old_data = mb_ir::SeqEntryData {
            clip_idx: old.clip_idx,
            length: old.length,
            termination: old.termination,
        };
        let forward = Edit::SetSeqEntry { track: track_idx as u16, beat, entry: None };
        let reverse = Edit::SetSeqEntry { track: track_idx as u16, beat, entry: Some(old_data) };
        self.apply_edit(forward.clone());
        Some((forward, reverse))
    }

    /// Read helper: get the sequence entry at a specific beat.
    pub fn seq_entry_at(&self, track_idx: usize, beat: u32) -> Option<&mb_ir::SeqEntry> {
        self.song.tracks.get(track_idx)?.seq_entry_at_beat(beat)
    }

    /// Toggle mute state on a track. Sends bypass to audio thread for live mute.
    pub fn toggle_track_mute(&mut self, track_idx: usize) {
        let Some(track) = self.song.tracks.get_mut(track_idx) else { return };
        track.muted = !track.muted;
        let muted = track.muted;
        let node_id = track.machine_node;
        if let Some(node_id) = node_id {
            self.push_edit(Edit::SetNodeBypass { node: node_id, bypassed: muted });
        }
    }

    // --- Edit dispatch ---

    /// Apply an edit to the local song and push it to the audio thread if playing.
    pub fn apply_edit(&mut self, edit: Edit) {
        apply_edit_to_song(&mut self.song, &edit);
        self.push_edit(edit);
    }

    /// Push an edit to the audio thread (if playing).
    fn push_edit(&mut self, edit: Edit) {
        if let Some(pb) = &mut self.playback {
            let _ = pb.edit_producer.try_push(edit);
        }
    }

    // --- Real-time playback ---

    pub fn play(&mut self) {
        self.play_song(self.song.clone());
    }

    pub fn play_pattern(&mut self, track_idx: usize, clip_idx: usize) {
        self.play_song(self.single_clip_song(track_idx, clip_idx as u16));
    }

    fn play_song(&mut self, song: Song) {
        self.stop();

        // Collect initial mute state before song is moved to audio thread
        let initial_bypasses: Vec<_> = song.tracks.iter()
            .filter(|t| t.muted)
            .filter_map(|t| t.machine_node)
            .collect();

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

        let mut pb = PlaybackHandle {
            stop_signal,
            current_time,
            finished,
            thread: Some(thread),
            edit_producer,
        };

        // Send initial bypass state for tracks muted before play
        for node_id in initial_bypasses {
            let _ = pb.edit_producer.try_push(Edit::SetNodeBypass { node: node_id, bypassed: true });
        }

        self.playback = Some(pb);
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
    pub fn track_position(&self, track_idx: usize) -> Option<TrackPlaybackPosition> {
        let pb = self.playback.as_ref()?;
        if pb.finished.load(Ordering::Relaxed) {
            return None;
        }
        let packed = pb.current_time.load(Ordering::Relaxed);
        let time = unpack_time(packed);
        time_to_track_position(&self.song, time, track_idx)
    }

    // --- Offline rendering ---

    pub fn render_frames(&self, sample_rate: u32, max_frames: usize) -> Vec<[f32; 2]> {
        render_song_frames(self.song.clone(), sample_rate, max_frames)
    }

    pub fn render_to_wav(&self, sample_rate: u32, max_seconds: u32) -> Vec<u8> {
        render_song_to_wav(self.song.clone(), sample_rate, max_seconds)
    }

    pub fn render_pattern_to_wav(&self, track_idx: usize, clip_idx: usize, sample_rate: u32, max_seconds: u32) -> Vec<u8> {
        render_song_to_wav(self.single_clip_song(track_idx, clip_idx as u16), sample_rate, max_seconds)
    }

    // --- Helpers ---

    /// Build a song that plays only the given clip on the given track.
    fn single_clip_song(&self, track_idx: usize, clip_idx: u16) -> Song {
        let mut song = self.song.clone();
        rebuild_track_sequences(&mut song, track_idx, clip_idx);
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
        Edit::SetNodeBypass { .. } => {} // Handled by engine directly
        Edit::SetSeqEntry { track, beat, entry } => {
            apply_set_seq_entry(song, *track, *beat, entry);
        }
    }
}

/// Apply a SetSeqEntry edit: remove any entry at beat, optionally insert new one.
fn apply_set_seq_entry(song: &mut Song, track_idx: u16, beat: u32, entry: &Option<mb_ir::SeqEntryData>) {
    let Some(track) = song.tracks.get_mut(track_idx as usize) else { return };
    // Remove existing entry at this beat
    track.sequence.retain(|e| e.start.beat as u32 != beat);
    // Insert new entry if provided
    if let Some(data) = entry {
        let new_entry = mb_ir::SeqEntry {
            start: mb_ir::MusicalTime::from_beats(beat as u64),
            clip_idx: data.clip_idx,
            length: data.length,
            termination: data.termination,
        };
        // Insert sorted by start time
        let pos = track.sequence.iter()
            .position(|e| e.start > new_entry.start)
            .unwrap_or(track.sequence.len());
        track.sequence.insert(pos, new_entry);
    }
}

/// Rebuild track sequences to play only a single clip on a single track.
fn rebuild_track_sequences(song: &mut Song, track_idx: usize, clip_idx: u16) {
    use mb_ir::SeqEntry;
    let length = song.tracks.get(track_idx)
        .and_then(|t| t.get_pattern_at(clip_idx as usize))
        .map_or(0, |p| p.rows);
    let entry = SeqEntry { start: mb_ir::MusicalTime::zero(), clip_idx, length, termination: mb_ir::SeqTermination::Natural };
    for (i, track) in song.tracks.iter_mut().enumerate() {
        track.sequence = if i == track_idx && (clip_idx as usize) < track.clips.len() {
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
        engine.render_block(&mut batch[..n]);

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

/// Check if placing a clip of the given length at the given beat would overlap
/// any existing sequence entry (excluding an entry already at that beat).
fn would_overlap(track: &mb_ir::Track, beat: u32, length: u16, rpb: u8) -> bool {
    let new_start = mb_ir::MusicalTime::from_beats(beat as u64);
    let new_end = new_start.add_rows(length as u32, rpb as u32);
    track.sequence.iter()
        .filter(|e| e.start.beat as u32 != beat) // skip entry we'd replace
        .any(|e| {
            let pat_rpb = track.get_pattern_at(e.clip_idx as usize)
                .and_then(|p| p.rows_per_beat)
                .map_or(rpb as u32, |r| r as u32);
            let e_end = e.start.add_rows(e.length as u32, pat_rpb);
            // Overlap if ranges intersect
            new_start < e_end && e.start < new_end
        })
}

/// End time of a track's sequence (after the last clip finishes).
fn track_end_time(song: &Song, track_idx: usize) -> mb_ir::MusicalTime {
    let rpb = song.rows_per_beat as u32;
    song.tracks.get(track_idx)
        .and_then(|t| {
            let last = t.sequence.last()?;
            let pat_rpb = t.clips.get(last.clip_idx as usize)
                .and_then(|c| c.pattern())
                .and_then(|p| p.rows_per_beat)
                .map_or(rpb, |r| r as u32);
            Some(last.start.add_rows(last.length as u32, pat_rpb))
        })
        .unwrap_or(mb_ir::MusicalTime::zero())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a controller with a simple song: 1 track, 2 clips (16 rows each), rpb=4.
    fn test_controller() -> Controller {
        let mut ctrl = Controller::new();
        ctrl.new_song(4);
        ctrl.add_clip(0, 16); // clip 1 (clip 0 already created by new_song)
        ctrl
    }

    #[test]
    fn set_seq_entry_inserts_sorted() {
        let mut ctrl = test_controller();
        // Clip 0 is 64 rows at rpb=4 = 16 beats. Place clip 1 at beat 16 (after clip 0).
        let result = ctrl.set_seq_entry(0, 16, 1);
        assert!(result.is_some());
        let track = &ctrl.song().tracks[0];
        assert_eq!(track.sequence.len(), 2);
        assert_eq!(track.sequence[0].start.beat, 0);
        assert_eq!(track.sequence[1].start.beat, 16);
        assert_eq!(track.sequence[1].clip_idx, 1);
    }

    #[test]
    fn remove_seq_entry_at_beat() {
        let mut ctrl = test_controller();
        // Remove the default entry at beat 0
        let result = ctrl.remove_seq_entry(0, 0);
        assert!(result.is_some());
        assert!(ctrl.song().tracks[0].sequence.is_empty());
    }

    #[test]
    fn remove_seq_entry_nonexistent_returns_none() {
        let ctrl = test_controller();
        // Nothing at beat 99
        assert!(ctrl.seq_entry_at(0, 99).is_none());
    }

    #[test]
    fn overlap_rejection() {
        let mut ctrl = test_controller();
        // Clip 0 is 64 rows at beat 0 with rpb=4 = 16 beats.
        // Placing at beat 4 overlaps.
        let result = ctrl.set_seq_entry(0, 4, 0);
        assert!(result.is_none(), "Should reject overlapping placement");
    }

    #[test]
    fn overlap_allows_adjacent() {
        let mut ctrl = test_controller();
        // Clip 1 is 16 rows = 4 beats. Placing right after clip 0 (beat 16) should work.
        let result = ctrl.set_seq_entry(0, 16, 1);
        assert!(result.is_some(), "Adjacent placement should succeed");
    }

    #[test]
    fn set_seq_entry_undo_round_trip() {
        let mut ctrl = test_controller();
        // Remove entry at beat 0 first to have a clean slate
        ctrl.remove_seq_entry(0, 0);
        assert!(ctrl.song().tracks[0].sequence.is_empty());

        // Place clip 0 at beat 0
        let (fwd, rev) = ctrl.set_seq_entry(0, 0, 0).unwrap();
        assert_eq!(ctrl.song().tracks[0].sequence.len(), 1);

        // Undo (apply reverse)
        ctrl.apply_edit(rev);
        assert!(ctrl.song().tracks[0].sequence.is_empty());

        // Redo (apply forward)
        ctrl.apply_edit(fwd);
        assert_eq!(ctrl.song().tracks[0].sequence.len(), 1);
    }

    #[test]
    fn seq_entry_at_beat_lookup() {
        let ctrl = test_controller();
        let entry = ctrl.seq_entry_at(0, 0);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().clip_idx, 0);
        assert!(ctrl.seq_entry_at(0, 99).is_none());
    }

    #[test]
    fn would_overlap_no_conflict() {
        let mut track = mb_ir::Track::new(None, 0, 4);
        track.clips.push(mb_ir::Clip::Pattern(mb_ir::Pattern::new(16, 4)));
        track.sequence.push(mb_ir::SeqEntry {
            start: mb_ir::MusicalTime::zero(),
            clip_idx: 0,
            length: 16,
            termination: mb_ir::SeqTermination::Natural,
        });
        // Place after the first clip ends (beat 4 with rpb=4)
        assert!(!would_overlap(&track, 4, 16, 4));
    }

    #[test]
    fn would_overlap_detects_conflict() {
        let mut track = mb_ir::Track::new(None, 0, 4);
        track.clips.push(mb_ir::Clip::Pattern(mb_ir::Pattern::new(16, 4)));
        track.sequence.push(mb_ir::SeqEntry {
            start: mb_ir::MusicalTime::zero(),
            clip_idx: 0,
            length: 16,
            termination: mb_ir::SeqTermination::Natural,
        });
        // Place overlapping the first clip
        assert!(would_overlap(&track, 2, 16, 4));
    }
}
