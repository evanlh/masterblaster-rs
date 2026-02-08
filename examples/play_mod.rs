//! Plays a MOD file through the default audio device.
//!
//! Usage: cargo run --example play_mod -- path/to/file.mod

use mb_audio::{AudioOutput, CpalOutput};
use mb_engine::{Engine, Frame};
use mb_formats::load_mod;
use std::io::Write;
use std::{env, fs, thread, time::Duration};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: cargo run --example play_mod -- <file.mod>");
        std::process::exit(1);
    });

    // Load and parse
    let data = fs::read(&path).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {}", path, e);
        std::process::exit(1);
    });

    let song = load_mod(&data).unwrap_or_else(|e| {
        eprintln!("Failed to parse MOD: {:?}", e);
        std::process::exit(1);
    });

    println!("Title:    {}", song.title);
    println!("Channels: {}", song.channels.len());
    println!("Patterns: {}", song.patterns.len());
    println!("Orders:   {}", song.order.len());
    println!("Tempo:    {} BPM, Speed: {}", song.initial_tempo, song.initial_speed);

    let samples_with_data = song.samples.iter().filter(|s| !s.is_empty()).count();
    println!("Samples:  {} (with data)", samples_with_data);
    println!();

    // Set up audio output
    let (mut output, consumer) = CpalOutput::new().unwrap_or_else(|e| {
        eprintln!("Failed to initialize audio: {}", e);
        std::process::exit(1);
    });

    let sample_rate = output.sample_rate();
    println!("Sample rate: {} Hz", sample_rate);

    // Set up engine
    let mut engine = Engine::new(song, sample_rate);
    engine.schedule_song();
    engine.play();

    // Start audio stream
    output.build_stream(consumer).unwrap_or_else(|e| {
        eprintln!("Failed to start audio stream: {}", e);
        std::process::exit(1);
    });
    output.start().unwrap();

    println!("Playing... (press Ctrl+C to stop)");
    println!();

    // Feed frames to the audio output in a loop
    let frames_per_batch = sample_rate as usize / 100; // 10ms batches
    let mut batch = vec![Frame::silence(); frames_per_batch];

    loop {
        for frame in batch.iter_mut() {
            *frame = engine.render_frame();
        }
        let _ = output.write(&batch);

        // Print position
        let pos = engine.position();
        print!("\rTick: {:>8}", pos.tick);
        let _ = std::io::stdout().flush();

        // Sleep a bit to avoid busy-spinning
        thread::sleep(Duration::from_millis(5));
    }
}
