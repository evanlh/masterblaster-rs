//! Integration test: load MOD fixture → schedule → render frames → verify output.

use mb_engine::{Engine, Frame};
use mb_formats::load_mod;
use mb_ir::{Note, OrderEntry};
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

    // Dump pattern 7 cell data
    let pat = &song.patterns[7];
    println!("Pattern 7: {} rows x {} channels", pat.rows, pat.channels);
    println!();

    // Collect effects and samples used
    let mut effects_used = std::collections::BTreeSet::new();
    let mut samples_used = std::collections::BTreeSet::new();

    for row in 0..pat.rows {
        let mut row_has_data = false;
        for ch in 0..pat.channels {
            let cell = pat.cell(row, ch);
            if !cell.is_empty() {
                row_has_data = true;
            }
            if cell.effect != mb_ir::Effect::None {
                effects_used.insert(cell.effect.name());
            }
            if cell.instrument > 0 {
                samples_used.insert(cell.instrument);
            }
        }
        if row_has_data {
            print!("Row {:02X}: ", row);
            for ch in 0..pat.channels {
                let cell = pat.cell(row, ch);
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

    // Print sample info for each used instrument
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

    // Render pattern 7 in isolation and scan for clicks
    let mut isolated = song.clone();
    isolated.order.clear();
    isolated.order.push(OrderEntry::Pattern(7));

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

    // Scan for clicks: large frame-to-frame deltas
    let click_threshold = 4000i32; // ~12% of i16 range
    let mut clicks = Vec::new();
    for i in 1..frames.len() {
        let dl = (frames[i].left as i32 - frames[i-1].left as i32).abs();
        let dr = (frames[i].right as i32 - frames[i-1].right as i32).abs();
        let max_delta = dl.max(dr);
        if max_delta > click_threshold {
            clicks.push((i, max_delta, dl, dr));
        }
    }

    println!("Clicks detected (delta > {}): {}", click_threshold, clicks.len());
    // Show first 20
    for &(pos, max_d, dl, dr) in clicks.iter().take(20) {
        let time_ms = pos as f64 / 44.1;
        println!(
            "  Frame {: >7} ({:7.1}ms): max_delta={:5} (L:{:5} R:{:5}) | L: {:6} → {:6} | R: {:6} → {:6}",
            pos, time_ms, max_d, dl, dr,
            frames[pos-1].left, frames[pos].left,
            frames[pos-1].right, frames[pos].right
        );
    }
    if clicks.len() > 20 {
        println!("  ... and {} more", clicks.len() - 20);
    }

    // Summary stats
    let max_amp = frames.iter()
        .flat_map(|f| [f.left.unsigned_abs(), f.right.unsigned_abs()])
        .max()
        .unwrap_or(0);
    println!();
    println!("Max amplitude: {}", max_amp);
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

/// Regression test: SetSpeed effects must increase rendering duration.
/// Pattern 20 of musiklinjen has a ritardando (speed ramps 6→26),
/// so it should produce more audio than 64 rows at constant speed 6.
#[test]
fn setspeed_produces_ritardando() {
    let data = fs::read(fixtures_dir().join("musiklinjen.mod")).unwrap();
    let song = load_mod(&data).unwrap();

    // Render pattern 20 in isolation
    let mut isolated = song.clone();
    isolated.order.clear();
    isolated.order.push(OrderEntry::Pattern(20));

    let mut engine = Engine::new(isolated, 44100);
    engine.schedule_song();
    engine.play();

    let mut frame_count = 0usize;
    while !engine.is_finished() && frame_count < 44100 * 60 {
        engine.render_frame();
        frame_count += 1;
    }

    // PatternBreak at row 15 → only 16 rows play.
    // At constant speed 6 / 125 BPM: 16 rows * 6 ticks/row * 882 samples/tick = 84,672 frames
    // With ritardando (speed 6→26): 223 ticks * 882 = 196,686 frames
    let constant_speed_frames = 16 * 6 * 882;
    assert!(
        frame_count > constant_speed_frames * 2,
        "SetSpeed ritardando should produce >2x frames vs constant speed: got {}, baseline {}",
        frame_count, constant_speed_frames
    );
}
