//! Integration tests for MOD parser against real fixture files.

use mb_formats::load_mod;
use mb_ir::{Note, OrderEntry};
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

fn count_notes(song: &mb_ir::Song) -> usize {
    song.patterns
        .iter()
        .map(|pat| {
            (0..pat.rows)
                .flat_map(|row| (0..pat.channels).map(move |ch| (row, ch)))
                .filter(|&(row, ch)| matches!(pat.cell(row, ch).note, Note::On(_)))
                .count()
        })
        .sum()
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

    // 4-channel MOD
    assert_eq!(song.channels.len(), 4);

    // All patterns: 64 rows, 4 channels
    for (i, pat) in song.patterns.iter().enumerate() {
        assert_eq!(pat.rows, 64, "Pattern {} rows", i);
        assert_eq!(pat.channels, 4, "Pattern {} channels", i);
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

    // Order list references only existing patterns
    let num_patterns = song.patterns.len();
    for (i, entry) in song.order.iter().enumerate() {
        if let OrderEntry::Pattern(idx) = entry {
            assert!((*idx as usize) < num_patterns, "Order {} -> pattern {}", i, idx);
        }
    }
}

// --- kawaik1.mod ---

#[test]
fn kawaik1_structure() {
    let song = load_fixture("kawaik1.mod");
    assert_mod_invariants(&song);

    assert_eq!(song.title.as_str(), "kawai-k1");
    assert_eq!(song.patterns.len(), 9);
    assert_eq!(song.order.len(), 20);
    assert_eq!(count_samples_with_data(&song), 9);
    assert_eq!(count_notes(&song), 538);
}

#[test]
fn kawaik1_order_list() {
    let song = load_fixture("kawaik1.mod");

    let orders: Vec<u8> = song.order.iter().map(|o| match o {
        OrderEntry::Pattern(i) => *i,
        _ => panic!("unexpected non-pattern order entry"),
    }).collect();

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
    assert_eq!(song.patterns.len(), 10);
    assert_eq!(song.order.len(), 15);
    assert_eq!(count_samples_with_data(&song), 10);
    assert_eq!(count_notes(&song), 995);
}

#[test]
fn noise_synth_pop_order_list() {
    let song = load_fixture("noise_synth_pop.mod");

    let orders: Vec<u8> = song.order.iter().map(|o| match o {
        OrderEntry::Pattern(i) => *i,
        _ => panic!("unexpected non-pattern order entry"),
    }).collect();

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
