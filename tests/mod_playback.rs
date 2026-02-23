//! Integration test: load MOD fixture → schedule → render frames → verify output.

use mb_engine::Engine;
use mb_formats::load_mod;
use mb_ir::Note;
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

fn has_nonsilent_frames(frames: &[[f32; 2]]) -> bool {
    frames.iter().any(|f| f[0] != 0.0 || f[1] != 0.0)
}

fn max_amplitude(frames: &[[f32; 2]]) -> f32 {
    frames
        .iter()
        .flat_map(|f| [f[0].abs(), f[1].abs()])
        .fold(0.0f32, f32::max)
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
            frame[0] >= -1.0 && frame[0] <= 1.0,
            "Frame {} left out of range: {}",
            i,
            frame[0]
        );
        assert!(
            frame[1] >= -1.0 && frame[1] <= 1.0,
            "Frame {} right out of range: {}",
            i,
            frame[1]
        );
    }
}

#[test]
fn kawaik1_has_meaningful_amplitude() {
    let mut engine = load_and_schedule("kawaik1.mod", 44100);
    let frames = engine.render_frames(44100);
    let max = max_amplitude(&frames);
    // 100/32768 ≈ 0.003
    assert!(max > 0.003, "Max amplitude {} too low for real MOD playback", max);
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
    assert!(max > 0.003, "Max amplitude {} too low", max);
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
        pos_after > pos_before,
        "Position should advance: before={:?}, after={:?}",
        pos_before,
        pos_after
    );
}

#[test]
fn musiklinjen_pattern7_diagnostics() {
    let data = fs::read(fixtures_dir().join("musiklinjen.mod")).unwrap();
    let song = load_mod(&data).unwrap();

    // Get pattern 7 data from the first track's clip 7
    let track = &song.tracks[0];
    let pat = track.clips[7].pattern().unwrap();
    println!("Clip 7 (track 0): {} rows x {} channels", pat.rows, pat.channels);
    println!();

    // Collect effects and samples used across all tracks' clip 7
    let mut effects_used = std::collections::BTreeSet::new();
    let mut samples_used = std::collections::BTreeSet::new();

    for t in &song.tracks {
        if let Some(p) = t.clips.get(7).and_then(|c| c.pattern()) {
            for row in 0..p.rows {
                let cell = p.cell(row, 0);
                if cell.effect != mb_ir::Effect::None {
                    effects_used.insert(cell.effect.name());
                }
                if cell.instrument > 0 {
                    samples_used.insert(cell.instrument);
                }
            }
        }
    }

    // Print rows with data (all 4 tracks' clip 7, side by side)
    for row in 0..64u16 {
        let mut row_has_data = false;
        for t in &song.tracks {
            if let Some(p) = t.clips.get(7).and_then(|c| c.pattern()) {
                if !p.cell(row, 0).is_empty() {
                    row_has_data = true;
                }
            }
        }
        if row_has_data {
            print!("Row {:02X}: ", row);
            for t in &song.tracks {
                let cell = t.clips.get(7)
                    .and_then(|c| c.pattern())
                    .map(|p| p.cell(row, 0))
                    .copied()
                    .unwrap_or(mb_ir::Cell::empty());
                let note_str = match cell.note {
                    Note::On(n) => {
                        let names = ["C-","C#","D-","D#","E-","F-","F#","G-","G#","A-","A#","B-"];
                        format!("{}{}", names[(n % 12) as usize], n / 12)
                    }
                    Note::Off => "OFF".to_string(),
                    _ => "...".to_string(),
                };
                let inst_str = if cell.instrument > 0 {
                    format!("{:02X}", cell.instrument)
                } else {
                    "..".to_string()
                };
                let fx_str = if cell.effect != mb_ir::Effect::None {
                    format!("{:?}", cell.effect)
                } else {
                    "...".to_string()
                };
                print!("| {} {} {} ", note_str, inst_str, fx_str);
            }
            println!("|");
        }
    }

    println!();
    println!("Effects used: {:?}", effects_used);
    println!("Instruments used: {:?}", samples_used);

    for &inst in &samples_used {
        let idx = (inst - 1) as usize;
        if let Some(sample) = song.samples.get(idx) {
            println!(
                "  Sample {:02X} '{}': len={}, loop={}..{}, has_loop={}, vol={}, c4={}",
                inst, sample.name, sample.len(), sample.loop_start, sample.loop_end,
                sample.has_loop(), sample.default_volume, sample.c4_speed
            );
        }
    }

    // Render clip 7 in isolation — rebuild sequences to play only clip 7
    let mut isolated = song.clone();
    let entry = mb_ir::SeqEntry { start: mb_ir::MusicalTime::zero(), clip_idx: 7 };
    for track in &mut isolated.tracks {
        track.sequence = if track.clips.len() > 7 {
            vec![entry]
        } else {
            Vec::new()
        };
    }

    let mut engine = Engine::new(isolated, 44100);
    engine.schedule_song();
    engine.play();

    let max_frames = 44100 * 10;
    let mut frames = Vec::with_capacity(max_frames);
    while !engine.is_finished() && frames.len() < max_frames {
        frames.push(engine.render_frame());
    }
    println!();
    println!("Rendered {} frames ({:.2}s)", frames.len(), frames.len() as f64 / 44100.0);

    let click_threshold = 4000.0 / 32768.0;
    let mut clicks = Vec::new();
    for i in 1..frames.len() {
        let dl = (frames[i][0] - frames[i-1][0]).abs();
        let dr = (frames[i][1] - frames[i-1][1]).abs();
        let max_delta = dl.max(dr);
        if max_delta > click_threshold {
            clicks.push((i, max_delta, dl, dr));
        }
    }

    println!("Clicks detected (delta > {:.4}): {}", click_threshold, clicks.len());
    for &(pos, max_d, dl, dr) in clicks.iter().take(20) {
        let time_ms = pos as f64 / 44.1;
        println!(
            "  Frame {: >7} ({:7.1}ms): max_delta={:.5} (L:{:.5} R:{:.5}) | L: {:.5} → {:.5} | R: {:.5} → {:.5}",
            pos, time_ms, max_d, dl, dr,
            frames[pos-1][0], frames[pos][0],
            frames[pos-1][1], frames[pos][1]
        );
    }
    if clicks.len() > 20 {
        println!("  ... and {} more", clicks.len() - 20);
    }

    let max_amp = frames.iter()
        .flat_map(|f| [f[0].abs(), f[1].abs()])
        .fold(0.0f32, f32::max);
    println!();
    println!("Max amplitude: {:.5}", max_amp);
}

#[test]
fn stop_produces_silence() {
    let mut engine = load_and_schedule("kawaik1.mod", 44100);
    engine.render_frames(1000);
    engine.stop();

    let frames = engine.render_frames(100);
    assert!(
        !has_nonsilent_frames(&frames),
        "Expected silence after stop"
    );
}

/// Regression test: SetSpeed effects must increase rendering duration.
/// Pattern 20 of musiklinjen has a ritardando (speed ramps 6→26),
/// so it should produce more audio than 64 rows at constant speed 6.
#[test]
fn setspeed_produces_ritardando() {
    let data = fs::read(fixtures_dir().join("musiklinjen.mod")).unwrap();
    let song = load_mod(&data).unwrap();

    // Render clip 20 in isolation
    let mut isolated = song.clone();
    let entry = mb_ir::SeqEntry { start: mb_ir::MusicalTime::zero(), clip_idx: 20 };
    for track in &mut isolated.tracks {
        track.sequence = if track.clips.len() > 20 {
            vec![entry]
        } else {
            Vec::new()
        };
    }

    let mut engine = Engine::new(isolated, 44100);
    engine.schedule_song();
    engine.play();

    let mut frame_count = 0usize;
    while !engine.is_finished() && frame_count < 44100 * 60 {
        engine.render_frame();
        frame_count += 1;
    }

    let constant_speed_frames = 16 * 6 * 882;
    assert!(
        frame_count > constant_speed_frames * 2,
        "SetSpeed ritardando should produce >2x frames vs constant speed: got {}, baseline {}",
        frame_count, constant_speed_frames
    );
}
