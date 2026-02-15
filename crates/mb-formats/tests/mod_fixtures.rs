//! Integration tests for MOD parser against real fixture files.

use mb_formats::load_mod;
use mb_ir::Note;
use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mod")
}

fn load_fixture(name: &str) -> mb_ir::Song {
    let path = fixtures_dir().join(name);
    let data =
        fs::read(&path).unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));
    load_mod(&data).unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", name, e))
}

/// Count notes across all track clips (each track has single-column clips).
fn count_notes(song: &mb_ir::Song) -> usize {
    // Each multi-channel pattern was split into N single-column clips.
    // To count total notes, iterate the clip pool of each track.
    // Since clips are duplicated across tracks (one column per track),
    // we just need to iterate all tracks' clips and count notes.
    song.tracks.iter()
        .flat_map(|t| t.clips.iter())
        .filter_map(|c| c.pattern())
        .flat_map(|pat| (0..pat.rows).map(move |row| pat.cell(row, 0)))
        .filter(|cell| matches!(cell.note, Note::On(_)))
        .count()
}

/// Number of unique clips (from first track — all tracks have the same clip count).
fn clip_count(song: &mb_ir::Song) -> usize {
    song.tracks.first().map(|t| t.clips.len()).unwrap_or(0)
}

/// Sequence length (from first track).
fn seq_len(song: &mb_ir::Song) -> usize {
    song.tracks.first().map(|t| t.sequence.len()).unwrap_or(0)
}

/// Get the sequence as clip indices (from first track).
fn seq_clip_indices(song: &mb_ir::Song) -> Vec<u16> {
    song.tracks.first()
        .map(|t| t.sequence.iter().map(|e| e.clip_idx).collect())
        .unwrap_or_default()
}

fn count_samples_with_data(song: &mb_ir::Song) -> usize {
    song.samples.iter().filter(|s| !s.is_empty()).count()
}

fn assert_mod_invariants(song: &mb_ir::Song) {
    // All MOD files have 31 sample/instrument slots
    assert_eq!(song.samples.len(), 31);
    assert_eq!(song.instruments.len(), 31);

    // MOD defaults
    assert_eq!(song.initial_tempo, 125);
    assert_eq!(song.initial_speed, 6);

    // 4-channel MOD → 4 tracks
    assert_eq!(song.channels.len(), 4);
    assert_eq!(song.tracks.len(), 4);

    // All clips: 64 rows, 1 channel (single-column after split)
    for (ti, track) in song.tracks.iter().enumerate() {
        for (ci, clip) in track.clips.iter().enumerate() {
            if let Some(pat) = clip.pattern() {
                assert_eq!(pat.rows, 64, "Track {} clip {} rows", ti, ci);
                assert_eq!(pat.channels, 1, "Track {} clip {} channels", ti, ci);
            }
        }
    }

    // All sample volumes in range
    for (i, sample) in song.samples.iter().enumerate() {
        assert!(sample.default_volume <= 64, "Sample {} volume {}", i, sample.default_volume);
    }

    // All loop bounds valid
    for (i, sample) in song.samples.iter().enumerate() {
        if sample.has_loop() {
            assert!(sample.loop_start < sample.loop_end, "Sample {} loop bounds", i);
            assert!(sample.loop_end <= sample.len() as u32, "Sample {} loop_end overflow", i);
        }
    }

    // Sequence entries reference only existing clips
    let num_clips = clip_count(song);
    for track in &song.tracks {
        for (i, entry) in track.sequence.iter().enumerate() {
            assert!(
                (entry.clip_idx as usize) < num_clips,
                "Track seq {} -> clip {} (only {} clips)",
                i, entry.clip_idx, num_clips
            );
        }
    }
}

// --- kawaik1.mod ---

#[test]
fn kawaik1_structure() {
    let song = load_fixture("kawaik1.mod");
    assert_mod_invariants(&song);

    assert_eq!(song.title.as_str(), "kawai-k1");
    assert_eq!(clip_count(&song), 9);
    assert_eq!(seq_len(&song), 20);
    assert_eq!(count_samples_with_data(&song), 9);
    assert_eq!(count_notes(&song), 538);
}

#[test]
fn kawaik1_order_list() {
    let song = load_fixture("kawaik1.mod");

    let orders: Vec<u16> = seq_clip_indices(&song);
    assert_eq!(orders, vec![3, 2, 4, 1, 1, 5, 5, 0, 0, 1, 1, 2, 5, 5, 7, 6, 1, 1, 0, 8]);
}

#[test]
fn kawaik1_named_samples() {
    let song = load_fixture("kawaik1.mod");

    assert_eq!(song.samples[2].name.as_str(), "st-06:snare");
    assert_eq!(song.samples[2].len(), 2752);
    assert_eq!(song.samples[2].default_volume, 64);

    assert_eq!(song.samples[4].name.as_str(), "st-06:becken");
    assert_eq!(song.samples[4].len(), 11458);

    assert_eq!(song.samples[6].name.as_str(), "st-06:stringx");
    assert_eq!(song.samples[6].len(), 12064);
    assert!(song.samples[6].has_loop());
}

#[test]
fn kawaik1_sample_lengths() {
    let song = load_fixture("kawaik1.mod");

    let expected: [(usize, u32); 9] = [
        (0, 15054), (1, 14126), (2, 2752), (3, 1684), (4, 11458),
        (5, 5848), (6, 12064), (7, 11292), (8, 12974),
    ];
    for (idx, len) in expected {
        assert_eq!(song.samples[idx].len() as u32, len, "Sample {} length", idx);
    }
}

// --- noise_synth_pop.mod ---

#[test]
fn noise_synth_pop_structure() {
    let song = load_fixture("noise_synth_pop.mod");
    assert_mod_invariants(&song);

    assert_eq!(song.title.as_str(), "noise synth pop");
    assert_eq!(clip_count(&song), 10);
    assert_eq!(seq_len(&song), 15);
    assert_eq!(count_samples_with_data(&song), 10);
    assert_eq!(count_notes(&song), 995);
}

#[test]
fn noise_synth_pop_order_list() {
    let song = load_fixture("noise_synth_pop.mod");

    let orders: Vec<u16> = seq_clip_indices(&song);
    assert_eq!(orders, vec![0, 1, 2, 3, 4, 5, 3, 4, 5, 6, 7, 3, 4, 8, 9]);
}

#[test]
fn noise_synth_pop_sample_lengths() {
    let song = load_fixture("noise_synth_pop.mod");

    let expected: [(usize, u32); 10] = [
        (0, 168), (1, 12568), (2, 1116), (3, 5622), (4, 510),
        (5, 180), (6, 180), (7, 180), (8, 180), (9, 2),
    ];
    for (idx, len) in expected {
        assert_eq!(song.samples[idx].len() as u32, len, "Sample {} length", idx);
    }
}
