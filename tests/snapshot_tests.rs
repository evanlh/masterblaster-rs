//! Snapshot tests: render MOD fixtures to WAV, compare against stored golden files.
//!
//! To update snapshots:
//!   UPDATE_SNAPSHOTS=1 cargo test --test snapshot_tests
//!
//! To verify against existing snapshots:
//!   cargo test --test snapshot_tests

use mb_formats::parse_wav_i16_samples;
use mb_master::Controller;
use std::path::PathBuf;
use std::{env, fs};

const SAMPLE_RATE: u32 = 44100;
const MAX_SECONDS: u32 = 10;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mod")
}

fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

fn should_update() -> bool {
    env::var("UPDATE_SNAPSHOTS").is_ok()
}

fn snapshot_path(name: &str) -> PathBuf {
    snapshots_dir().join(format!("{}.wav", name))
}

fn snapshot_stem(fixture_name: &str) -> String {
    fixture_name.strip_suffix(".mod").unwrap_or(fixture_name).to_string()
}

/// Compare WAV files with ±2 LSB tolerance on i16 samples.
fn assert_snapshot(name: &str, wav_bytes: &[u8]) {
    let path = snapshot_path(name);

    if should_update() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, wav_bytes).unwrap();
        println!("Updated snapshot: {}", path.display());
        return;
    }

    let expected = fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "Snapshot not found: {}\nRun with UPDATE_SNAPSHOTS=1 to generate.",
            path.display()
        )
    });

    let actual_samples = parse_wav_i16_samples(wav_bytes)
        .expect("Failed to parse actual WAV samples");
    let expected_samples = parse_wav_i16_samples(&expected)
        .expect("Failed to parse expected WAV samples");

    if actual_samples.len() != expected_samples.len() {
        panic!(
            "Snapshot mismatch: {}\n  expected samples: {}\n  actual samples:   {}\n\nRun with UPDATE_SNAPSHOTS=1 to update.",
            path.display(),
            expected_samples.len(),
            actual_samples.len(),
        );
    }

    // Tolerance of ±2 LSB accounts for:
    // - Integer right-shift (rounds toward -inf) vs f32 multiply (rounds toward zero)
    // - Two-stage conversion: i16→f32 in engine, f32→i16 in WAV writer
    const TOLERANCE: i32 = 2;
    for (i, (&actual, &expected)) in actual_samples.iter().zip(expected_samples.iter()).enumerate() {
        let diff = (actual as i32 - expected as i32).abs();
        if diff > TOLERANCE {
            panic!(
                "Snapshot mismatch: {}\n  sample {}: expected {}, got {} (diff {})\n  tolerance: ±{} LSB\n\nRun with UPDATE_SNAPSHOTS=1 to update.",
                path.display(), i, expected, actual, diff, TOLERANCE
            );
        }
    }
}

fn snapshot_test(fixture_name: &str) {
    let mut ctrl = Controller::new();
    ctrl.load_mod(&fs::read(fixtures_dir().join(fixture_name)).unwrap())
        .unwrap();
    let wav = ctrl.render_to_wav(SAMPLE_RATE, MAX_SECONDS);
    assert_snapshot(&snapshot_stem(fixture_name), &wav);
}

fn snapshot_pattern_test(fixture_name: &str, pattern: usize) {
    let mut ctrl = Controller::new();
    ctrl.load_mod(&fs::read(fixtures_dir().join(fixture_name)).unwrap())
        .unwrap();
    let wav = ctrl.render_pattern_to_wav(pattern, SAMPLE_RATE, MAX_SECONDS);
    let name = format!("{}_pat{:02}", snapshot_stem(fixture_name), pattern);
    assert_snapshot(&name, &wav);
}

#[test]
fn snapshot_kawaik1() {
    snapshot_test("kawaik1.mod");
}

#[test]
fn snapshot_noise_synth_pop() {
    snapshot_test("noise_synth_pop.mod");
}

#[test]
fn snapshot_musiklinjen() {
    snapshot_test("musiklinjen.mod");
}

#[test]
fn snapshot_musiklinjen_pat20_ritardando() {
    snapshot_pattern_test("musiklinjen.mod", 20);
}
