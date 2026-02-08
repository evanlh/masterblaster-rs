//! Snapshot tests: render MOD fixtures to WAV, compare against stored golden files.
//!
//! To update snapshots:
//!   UPDATE_SNAPSHOTS=1 cargo test --test snapshot_tests
//!
//! To verify against existing snapshots:
//!   cargo test --test snapshot_tests

use mb_engine::{Engine, Frame};
use mb_formats::load_mod;
use std::io::Write;
use std::path::PathBuf;
use std::{env, fs};

const SAMPLE_RATE: u32 = 44100;
const MAX_SECONDS: u32 = 10;
const MAX_FRAMES: usize = (SAMPLE_RATE * MAX_SECONDS) as usize;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mod")
}

fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

fn render_fixture(name: &str) -> Vec<Frame> {
    let data = fs::read(fixtures_dir().join(name)).unwrap();
    let song = load_mod(&data).unwrap();
    let mut engine = Engine::new(song, SAMPLE_RATE);
    engine.schedule_song();
    engine.play();

    let mut frames = Vec::with_capacity(MAX_FRAMES);
    while !engine.is_finished() && frames.len() < MAX_FRAMES {
        frames.push(engine.render_frame());
    }
    frames
}

fn frames_to_wav(frames: &[Frame], sample_rate: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    write_wav(&mut buf, frames, sample_rate).unwrap();
    buf
}

fn write_wav(w: &mut impl Write, frames: &[Frame], sample_rate: u32) -> std::io::Result<()> {
    let num_channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let block_align = num_channels * (bits_per_sample / 8);
    let data_size = frames.len() as u32 * block_align as u32;

    // RIFF header
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_size).to_le_bytes())?;
    w.write_all(b"WAVE")?;

    // fmt chunk
    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&num_channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&(sample_rate * block_align as u32).to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&bits_per_sample.to_le_bytes())?;

    // data chunk
    w.write_all(b"data")?;
    w.write_all(&data_size.to_le_bytes())?;
    for frame in frames {
        w.write_all(&frame.left.to_le_bytes())?;
        w.write_all(&frame.right.to_le_bytes())?;
    }
    Ok(())
}

fn should_update() -> bool {
    env::var("UPDATE_SNAPSHOTS").is_ok()
}

fn snapshot_path(fixture_name: &str) -> PathBuf {
    let stem = fixture_name.strip_suffix(".mod").unwrap_or(fixture_name);
    snapshots_dir().join(format!("{}.wav", stem))
}

fn assert_snapshot(fixture_name: &str, wav_bytes: &[u8]) {
    let path = snapshot_path(fixture_name);

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
    let frames = render_fixture(fixture_name);
    let wav = frames_to_wav(&frames, SAMPLE_RATE);
    assert_snapshot(fixture_name, &wav);
}

#[test]
fn snapshot_kawaik1() {
    snapshot_test("kawaik1.mod");
}

#[test]
fn snapshot_noise_synth_pop() {
    snapshot_test("noise_synth_pop.mod");
}
