//! masterblaster CLI — headless playback and WAV export.
//!
//! Usage:
//!   cargo cli path/to/file.mod
//!   cargo cli path/to/file.mod --wav output.wav
//!   cargo cli path/to/file.mod --pattern 0
//!   cargo cli path/to/file.mod --pattern 0 --wav output.wav

use mb_master::Controller;
use std::io::Write;
use std::{env, fs};

fn main() {
    let args: Vec<String> = env::args().collect();
    let path = args.get(1).unwrap_or_else(|| {
        eprintln!("Usage: mb-cli <file.mod> [--wav output.wav] [--pattern N]");
        std::process::exit(1);
    });

    let wav_path = args
        .iter()
        .position(|a| a == "--wav")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let pattern_idx: Option<usize> = args
        .iter()
        .position(|a| a == "--pattern")
        .map(|i| {
            args.get(i + 1)
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| {
                    eprintln!("--pattern requires a numeric argument");
                    std::process::exit(1);
                })
        });

    let data = fs::read(path).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {}", path, e);
        std::process::exit(1);
    });

    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut ctrl = Controller::new();
    let load_result = match ext.as_str() {
        "bmx" => ctrl.load_bmx(&data),
        _ => ctrl.load_mod(&data),
    };
    load_result.unwrap_or_else(|e| {
        eprintln!("Failed to parse {}: {:?}", ext.to_uppercase(), e);
        std::process::exit(1);
    });

    let song = ctrl.song();
    println!("Title:    {}", song.title);
    println!("Channels: {}", song.channels.len());
    println!("Tracks:   {}", song.tracks.len());

    let clip_count = song.tracks.first().map(|t| t.clips.len()).unwrap_or(0);
    let seq_len = song.tracks.first().map(|t| t.sequence.len()).unwrap_or(0);
    println!("Clips:    {}", clip_count);
    println!("Sequence: {} entries", seq_len);
    println!("Tempo:    {} BPM, Speed: {}", song.initial_tempo, song.initial_speed);

    let samples_with_data = song.samples.iter().filter(|s| !s.is_empty()).count();
    println!("Samples:  {} (with data)", samples_with_data);
    println!();

    if let Some(p) = pattern_idx {
        if p >= clip_count {
            eprintln!("Clip {} out of range (song has {})", p, clip_count);
            std::process::exit(1);
        }
        println!("\nClip: {}", p);

        if let Some(track) = song.tracks.first() {
            if let Some(pat) = track.clips[p].pattern() {
                let pf = mb_ir::analyze_pattern(pat);
                print!("{}", pf);
            }
        }
    }

    match (wav_path, pattern_idx) {
        (Some(wav), Some(p)) => render_to_wav_pattern(&ctrl, &wav, p),
        (Some(wav), None) => render_to_wav(&ctrl, &wav),
        (None, Some(p)) => play_pattern(&mut ctrl, p),
        (None, None) => play_audio(&mut ctrl),
    }
}

fn play_audio(ctrl: &mut Controller) {
    ctrl.play();
    println!("Playing...");
    println!();

    while ctrl.is_playing() {
        if let Some(pos) = ctrl.track_position(0) {
            print!(
                "\rSeq: {:02X} | Clip: {:02X} | Row: {:02X}",
                pos.seq_index, pos.clip_idx, pos.row
            );
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    println!("\rDone.          ");
}

fn play_pattern(ctrl: &mut Controller, pattern: usize) {
    ctrl.play_pattern(pattern);
    println!("Playing clip {}...", pattern);
    println!();

    while ctrl.is_playing() {
        if let Some(pos) = ctrl.track_position(0) {
            print!(
                "\rSeq: {:02X} | Clip: {:02X} | Row: {:02X}",
                pos.seq_index, pos.clip_idx, pos.row
            );
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    println!("\rDone.          ");
}

fn render_to_wav_pattern(ctrl: &Controller, path: &str, pattern: usize) {
    let sample_rate: u32 = 44100;
    let max_seconds: u32 = 1200;
    println!("Rendering clip {} to {} at {} Hz...", pattern, path, sample_rate);

    let wav = ctrl.render_pattern_to_wav(pattern, sample_rate, max_seconds);
    println!("Rendered {} bytes", wav.len());

    fs::write(path, &wav).unwrap_or_else(|e| {
        eprintln!("Failed to write {}: {}", path, e);
        std::process::exit(1);
    });

    println!("Done.");
}

fn render_to_wav(ctrl: &Controller, path: &str) {
    let sample_rate: u32 = 44100;
    let max_seconds: u32 = 1200;
    println!("Rendering to {} at {} Hz...", path, sample_rate);

    let wav = ctrl.render_to_wav(sample_rate, max_seconds);
    println!("Rendered {} bytes", wav.len());

    fs::write(path, &wav).unwrap_or_else(|e| {
        eprintln!("Failed to write {}: {}", path, e);
        std::process::exit(1);
    });

    println!("Done.");
}
