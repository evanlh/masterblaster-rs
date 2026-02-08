//! Plays a MOD file through the default audio device, or writes to WAV.
//!
//! Usage:
//!   cargo run --example play_mod -- path/to/file.mod
//!   cargo run --example play_mod -- path/to/file.mod --wav output.wav

use mb_audio::{AudioOutput, CpalOutput};
use mb_engine::{Engine, Frame};
use mb_formats::load_mod;
use std::io::Write;
use std::{env, fs};

fn main() {
    let args: Vec<String> = env::args().collect();
    let path = args.get(1).unwrap_or_else(|| {
        eprintln!("Usage: play_mod <file.mod> [--wav output.wav]");
        std::process::exit(1);
    });

    let wav_path = args
        .iter()
        .position(|a| a == "--wav")
        .and_then(|i| args.get(i + 1))
        .cloned();

    // Load and parse
    let data = fs::read(path).unwrap_or_else(|e| {
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

    match wav_path {
        Some(wav) => render_to_wav(song, &wav),
        None => play_audio(song),
    }
}

fn play_audio(song: mb_ir::Song) {
    let (mut output, consumer) = CpalOutput::new().unwrap_or_else(|e| {
        eprintln!("Failed to initialize audio: {}", e);
        std::process::exit(1);
    });

    let sample_rate = output.sample_rate();
    println!("Sample rate: {} Hz", sample_rate);

    let mut engine = Engine::new(song, sample_rate);
    engine.schedule_song();
    engine.play();

    output.build_stream(consumer).unwrap_or_else(|e| {
        eprintln!("Failed to start audio stream: {}", e);
        std::process::exit(1);
    });
    output.start().unwrap();

    println!("Playing...");
    println!();

    let print_interval = sample_rate as u64 / 100;
    let mut frame_count: u64 = 0;

    while !engine.is_finished() {
        let frame = engine.render_frame();
        output.write_spin(frame);

        frame_count += 1;
        if frame_count % print_interval == 0 {
            print!("\rTick: {:>8}", engine.position().tick);
            let _ = std::io::stdout().flush();
        }
    }

    // Drain: push a short tail of silence so the ring buffer flushes
    for _ in 0..sample_rate {
        output.write_spin(Frame::silence());
    }

    println!("\rDone.          ");
}

fn render_to_wav(song: mb_ir::Song, path: &str) {
    let sample_rate: u32 = 44100;
    let mut engine = Engine::new(song, sample_rate);
    engine.schedule_song();
    engine.play();

    println!("Rendering to {} at {} Hz...", path, sample_rate);

    // Render all frames
    let mut frames: Vec<Frame> = Vec::new();
    while !engine.is_finished() {
        frames.push(engine.render_frame());
    }

    println!("Rendered {} frames ({:.1}s)", frames.len(), frames.len() as f64 / sample_rate as f64);

    // Write WAV file (16-bit stereo PCM)
    let file = fs::File::create(path).unwrap_or_else(|e| {
        eprintln!("Failed to create {}: {}", path, e);
        std::process::exit(1);
    });
    let mut writer = std::io::BufWriter::new(file);

    write_wav(&mut writer, &frames, sample_rate).unwrap_or_else(|e| {
        eprintln!("Failed to write WAV: {}", e);
        std::process::exit(1);
    });

    println!("Done.");
}

fn write_wav(w: &mut impl Write, frames: &[Frame], sample_rate: u32) -> std::io::Result<()> {
    let num_channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let bytes_per_sample = bits_per_sample / 8;
    let block_align = num_channels * bytes_per_sample;
    let data_size = frames.len() as u32 * block_align as u32;
    let file_size = 36 + data_size;

    // RIFF header
    w.write_all(b"RIFF")?;
    w.write_all(&file_size.to_le_bytes())?;
    w.write_all(b"WAVE")?;

    // fmt chunk
    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?; // chunk size
    w.write_all(&1u16.to_le_bytes())?; // PCM format
    w.write_all(&num_channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&(sample_rate * block_align as u32).to_le_bytes())?; // byte rate
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
