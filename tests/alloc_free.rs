//! Allocation-free render path tests.
//!
//! These tests verify that `Engine::render_frame()` does not allocate
//! during the realtime phase. They render real fixture files for several
//! seconds to catch allocations triggered by specific effect combinations,
//! pattern breaks, or sample edge cases.
//!
//! Just run `cargo test` — no feature flags needed.

use assert_no_alloc::{assert_no_alloc, AllocDisabler};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

use mb_engine::Engine;
use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_mod(name: &str) -> mb_ir::Song {
    let data = fs::read(fixtures_dir().join("mod").join(name)).unwrap();
    mb_formats::load_mod(&data).unwrap()
}

fn load_bmx(name: &str) -> mb_ir::Song {
    let data = fs::read(fixtures_dir().join("bmx").join(name)).unwrap();
    mb_formats::load_bmx(&data).unwrap()
}

/// Render a song for `duration_frames`, aborting on any heap allocation.
fn assert_render_alloc_free(song: mb_ir::Song, duration_frames: usize) {
    let mut engine = Engine::new(song, 44100);
    engine.schedule_song();
    engine.play();

    assert_no_alloc(|| {
        for _ in 0..duration_frames {
            engine.render_frame();
        }
    });
}

#[test]
fn mod_elysium_alloc_free() {
    let song = load_mod("ELYSIUM.MOD");
    assert_render_alloc_free(song, 44100 * 5);
}

#[test]
fn mod_musiklinjen_alloc_free() {
    let song = load_mod("musiklinjen.mod");
    assert_render_alloc_free(song, 44100 * 5);
}

#[test]
fn bmx_tribal_alloc_free() {
    let song = load_bmx("tribal-60.bmx");
    assert_render_alloc_free(song, 44100 * 5);
}

#[test]
fn bmx_acousticelectro_alloc_free() {
    let song = load_bmx("acousticelectro-drumloop-100.bmx");
    assert_render_alloc_free(song, 44100 * 5);
}
