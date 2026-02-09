//! masterblaster CLI — headless playback and WAV export.
//!
//! Usage:
//!   cargo cli path/to/file.mod
//!   cargo cli path/to/file.mod --wav output.wav

use mb_master::Controller;
use std::io::Write;
use std::{env, fs};

fn main() {
    let args: Vec<String> = env::args().collect();
    let path = args.get(1).unwrap_or_else(|| {
        eprintln!("Usage: mb-cli <file.mod> [--wav output.wav]");
        std::process::exit(1);
    });

    let wav_path = args
        .iter()
        .position(|a| a == "--wav")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let data = fs::read(path).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {}", path, e);
        std::process::exit(1);
    });

    let mut ctrl = Controller::new();
    ctrl.load_mod(&data).unwrap_or_else(|e| {
        eprintln!("Failed to parse MOD: {:?}", e);
        std::process::exit(1);
    });

    let song = ctrl.song();
    println!("Title:    {}", song.title);
    println!("Channels: {}", song.channels.len());
    println!("Patterns: {}", song.patterns.len());
    println!("Orders:   {}", song.order.len());
    println!("Tempo:    {} BPM, Speed: {}", song.initial_tempo, song.initial_speed);

    let samples_with_data = song.samples.iter().filter(|s| !s.is_empty()).count();
    println!("Samples:  {} (with data)", samples_with_data);
    println!();

    let features = mb_ir::analyze(song);
    print!("{}", features);
    println!();

    match wav_path {
        Some(wav) => render_to_wav(&ctrl, &wav),
        None => play_audio(&mut ctrl),
    }
}

fn play_audio(ctrl: &mut Controller) {
    ctrl.play();
    println!("Playing...");
    println!();

    while ctrl.is_playing() {
        if let Some(pos) = ctrl.position() {
            print!(
                "\rOrd: {:02X} | Pat: {:02X} | Row: {:02X}",
                pos.order_index, pos.pattern_index, pos.row
            );
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    println!("\rDone.          ");
}

fn render_to_wav(ctrl: &Controller, path: &str) {
    let sample_rate: u32 = 44100;
    let max_seconds: u32 = 300;
    println!("Rendering to {} at {} Hz...", path, sample_rate);

    let wav = ctrl.render_to_wav(sample_rate, max_seconds);
    println!("Rendered {} bytes", wav.len());

    fs::write(path, &wav).unwrap_or_else(|e| {
        eprintln!("Failed to write {}: {}", path, e);
        std::process::exit(1);
    });

    println!("Done.");
}
