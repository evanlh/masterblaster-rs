//! Snapshot tests: render MOD fixtures to WAV, compare against stored golden files.
//!
//! To update snapshots:
//!   UPDATE_SNAPSHOTS=1 cargo test --test snapshot_tests
//!
//! To verify against existing snapshots:
//!   cargo test --test snapshot_tests

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

    if wav_bytes == expected.as_slice() {
        return;
    }

    let first_diff = wav_bytes
        .iter()
        .zip(expected.iter())
        .position(|(a, b)| a != b)
        .unwrap_or(wav_bytes.len().min(expected.len()));

    panic!(
        "Snapshot mismatch: {}\n  expected size: {} bytes\n  actual size:   {} bytes\n  first diff at byte: {}\n\nRun with UPDATE_SNAPSHOTS=1 to update.",
        path.display(),
        expected.len(),
        wav_bytes.len(),
        first_diff
    );
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
