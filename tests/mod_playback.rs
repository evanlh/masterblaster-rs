//! Integration test: load MOD fixture → schedule → render frames → verify output.

use mb_engine::{Engine, Frame};
use mb_formats::load_mod;
use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mod")
}

fn load_and_schedule(name: &str, sample_rate: u32) -> Engine {
    let path = fixtures_dir().join(name);
    let data = fs::read(&path).unwrap();
    let song = load_mod(&data).unwrap();
    let mut engine = Engine::new(song, sample_rate);
    engine.schedule_song();
    engine.play();
    engine
}

fn has_nonsilent_frames(frames: &[Frame]) -> bool {
    frames.iter().any(|f| f.left != 0 || f.right != 0)
}

fn max_amplitude(frames: &[Frame]) -> i16 {
    frames
        .iter()
        .flat_map(|f| [f.left.saturating_abs(), f.right.saturating_abs()])
        .max()
        .unwrap_or(0)
}

// --- kawaik1.mod ---

#[test]
fn kawaik1_renders_nonsilent() {
    let mut engine = load_and_schedule("kawaik1.mod", 44100);
    let frames = engine.render_frames(44100); // 1 second
    assert!(
        has_nonsilent_frames(&frames),
        "Expected non-silent output from kawaik1.mod"
    );
}

#[test]
fn kawaik1_output_within_range() {
    let mut engine = load_and_schedule("kawaik1.mod", 44100);
    let frames = engine.render_frames(44100);

    for (i, frame) in frames.iter().enumerate() {
        assert!(
            frame.left >= i16::MIN && frame.left <= i16::MAX,
            "Frame {} left out of range: {}",
            i,
            frame.left
        );
        assert!(
            frame.right >= i16::MIN && frame.right <= i16::MAX,
            "Frame {} right out of range: {}",
            i,
            frame.right
        );
    }
}

#[test]
fn kawaik1_has_meaningful_amplitude() {
    let mut engine = load_and_schedule("kawaik1.mod", 44100);
    let frames = engine.render_frames(44100);
    let max = max_amplitude(&frames);
    // Should be well above noise floor — real samples at volume 64
    assert!(max > 100, "Max amplitude {} too low for real MOD playback", max);
}

// --- noise_synth_pop.mod ---

#[test]
fn noise_synth_pop_renders_nonsilent() {
    let mut engine = load_and_schedule("noise_synth_pop.mod", 44100);
    let frames = engine.render_frames(44100);
    assert!(
        has_nonsilent_frames(&frames),
        "Expected non-silent output from noise_synth_pop.mod"
    );
}

#[test]
fn noise_synth_pop_has_meaningful_amplitude() {
    let mut engine = load_and_schedule("noise_synth_pop.mod", 44100);
    let frames = engine.render_frames(44100);
    let max = max_amplitude(&frames);
    assert!(max > 100, "Max amplitude {} too low", max);
}

// --- Engine behavior ---

#[test]
fn different_sample_rates_produce_output() {
    for rate in [22050, 44100, 48000] {
        let mut engine = load_and_schedule("kawaik1.mod", rate);
        let frames = engine.render_frames(rate as usize / 2); // 0.5 seconds
        assert!(
            has_nonsilent_frames(&frames),
            "No output at sample rate {}",
            rate
        );
    }
}

#[test]
fn playback_advances_position() {
    let mut engine = load_and_schedule("kawaik1.mod", 44100);
    let pos_before = engine.position();
    engine.render_frames(44100);
    let pos_after = engine.position();
    assert!(
        pos_after.tick > pos_before.tick,
        "Position should advance: before={}, after={}",
        pos_before.tick,
        pos_after.tick
    );
}

#[test]
fn stop_produces_silence() {
    let mut engine = load_and_schedule("kawaik1.mod", 44100);
    // Render a bit so channels are active
    engine.render_frames(1000);
    engine.stop();

    let frames = engine.render_frames(100);
    assert!(
        !has_nonsilent_frames(&frames),
        "Expected silence after stop"
    );
}
