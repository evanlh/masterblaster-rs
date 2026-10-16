//! Criterion benchmarks for the Faust render path.
//!
//! `mb-engine/benches/engine_bench.rs` only exercises the non-Faust engine core
//! (Tracker + AmigaFilter + passthrough). These benchmarks cover the Faust effect
//! machines that dominate BMX playback cost, in three shapes design 026 calls out:
//!
//! - **A1** — per-block `FaustMachine::render` cost (micro-benchmark).
//! - **deep chain** — a tracker feeding a long serial chain of Faust effects
//!   (no parallelism possible; the M1 "linear chain does not regress" case).
//! - **parallel branches** — a tracker fanning out into independent Faust effect
//!   branches (the M1 "parallel levels" case).
//!
//! Run with: `cargo bench -p mb-faust --bench faust_render`

use criterion::{criterion_group, criterion_main, Criterion};
use mb_engine::Engine;
use mb_faust::{create_faust_machine, faust_machine::FaustMachine};
use mb_ir::{
    build_tracks, AudioBuffer, AudioStream, Cell, ChannelSettings, Instrument, NodeType, Note,
    OrderEntry, Pattern, Sample, SampleData, Song, BLOCK_SIZE,
};

const SAMPLE_RATE: u32 = 44100;
const PATTERN_ROWS: u16 = 64;
const SINE_LEN: usize = 8000;
const NUM_CHANNELS: u8 = 8;
/// Blocks rendered per engine-benchmark iteration (256 frames ≈ 5.8 ms each).
const BLOCKS_PER_ITER: usize = 16;

/// The five Buzz machines with Faust implementations, in registry order.
const FX_NAMES: [&str; 5] = [
    "Jeskola Filter 2",
    "Jeskola Reverb 2",
    "Jeskola Freeverb",
    "FSM WahPro",
    "FSM WahPro2",
];

const FILTER2: &str = include_str!("../../../faust/filter2.dsp");
const REVERB2: &str = include_str!("../../../faust/reverb2.dsp");
const FREEVERB: &str = include_str!("../../../faust/reverb.dsp");
const WAHPRO: &str = include_str!("../../../faust/wahpro.dsp");
const WAH_PRO2: &str = include_str!("../../../faust/wahpro2.dsp");

const DSP_SOURCES: [(&str, &str); 5] = [
    ("filter2", FILTER2),
    ("reverb2", REVERB2),
    ("freeverb", FREEVERB),
    ("wahpro", WAHPRO),
    ("wahpro2", WAH_PRO2),
];

/// Generate a sine wave as i8 samples (no `rand` needed).
fn sine_wave(len: usize) -> Vec<i8> {
    (0..len)
        .map(|i| {
            let phase = (i as f64) * 2.0 * std::f64::consts::PI / 64.0;
            (phase.sin() * 127.0) as i8
        })
        .collect()
}

/// Deterministic pseudo-random note for a given row and channel.
fn pseudo_note(row: u16, channel: u8) -> Option<u8> {
    let hash = (row as u32)
        .wrapping_mul(31)
        .wrapping_add(channel as u32)
        .wrapping_mul(17);
    if hash % 4 == 0 {
        Some(36 + (hash % 48) as u8) // C-2 to B-5
    } else {
        None
    }
}

/// Build a pattern with deterministic NoteOn events spread across channels.
fn build_pattern(rows: u16, channels: u8) -> Pattern {
    let mut pat = Pattern::new(rows, channels);
    for row in 0..rows {
        for ch in 0..channels {
            if let Some(note) = pseudo_note(row, ch) {
                *pat.cell_mut(row, ch) = Cell {
                    note: Note::On(note),
                    instrument: 1,
                    ..Cell::empty()
                };
            }
        }
    }
    pat
}

/// A tracker machine node (the single generator that owns all channels).
fn tracker_node() -> NodeType {
    NodeType::Machine { machine_name: "Tracker".to_string(), is_tracker: true }
}

/// A named effect machine node (resolved to a Faust machine by name).
fn effect_node(name: &str) -> NodeType {
    NodeType::Machine { machine_name: name.to_string(), is_tracker: false }
}

/// Build a song whose tracker feeds the given effect names, either as a single
/// serial chain (`parallel == false`) or as independent parallel branches.
fn build_song(effect_names: &[&str], parallel: bool) -> Song {
    let mut song = Song::new("faust-bench");

    for i in 0..NUM_CHANNELS {
        song.channels.push(ChannelSettings {
            initial_pan: if i % 4 == 0 || i % 4 == 3 { -64 } else { 64 },
            initial_vol: 64,
            muted: false,
        });
    }

    let mut sample = Sample::new("sine");
    sample.data = SampleData::Mono8(sine_wave(SINE_LEN));
    sample.default_volume = 64;
    sample.c4_speed = 8363;
    song.samples.push(sample);

    let mut inst = Instrument::new("sine inst");
    inst.set_single_sample(0);
    song.instruments.push(inst);

    let tracker = song.graph.add_node(tracker_node());
    if parallel {
        for name in effect_names {
            let fx = song.graph.add_node(effect_node(name));
            song.graph.connect(tracker, fx);
            song.graph.connect(fx, 0);
        }
    } else {
        let mut prev = tracker;
        for name in effect_names {
            let fx = song.graph.add_node(effect_node(name));
            song.graph.connect(prev, fx);
            prev = fx;
        }
        song.graph.connect(prev, 0);
    }

    let pattern = build_pattern(PATTERN_ROWS, NUM_CHANNELS);
    let order = vec![OrderEntry::Pattern(0)];
    build_tracks(&mut song, &[pattern], &order);
    song
}

/// Build an engine for `effect_names` with the named machines replaced by Faust
/// implementations (mirrors `mb_master::build_machines`).
fn build_engine(effect_names: &[&str], parallel: bool) -> Engine {
    let song = build_song(effect_names, parallel);
    let mut machines = mb_engine::init_machines(&song, SAMPLE_RATE);
    for (i, node) in song.graph.nodes.iter().enumerate() {
        if let NodeType::Machine { machine_name, is_tracker: false } = &node.node_type {
            if let Some(mut m) = create_faust_machine(machine_name) {
                m.init(SAMPLE_RATE);
                machines[i] = Some(m);
            }
        }
    }
    let mut engine = Engine::with_machines(song, SAMPLE_RATE, machines);
    engine.schedule_song();
    engine.play();
    engine
}

/// A compile-once Faust machine for a named DSP source.
fn faust_machine(name: &str, source: &str) -> FaustMachine {
    FaustMachine::from_source_cached(name, source).expect("compile Faust DSP")
}

/// Benchmark a single `FaustMachine::render` call for `frames` frames.
fn bench_faust_render_one(c: &mut Criterion, id: &str, name: &str, source: &str, frames: u16) {
    let mut machine = faust_machine(name, source);
    let mut buf = AudioBuffer::new(2, frames);
    c.bench_function(&format!("faust_render_{id}_{frames}f"), |b| {
        b.iter(|| machine.render(&mut buf));
    });
}

/// Micro-benchmarks: per-call render cost for each DSP, at realtime block size
/// (256) and the offline 1-frame block where A1's per-call cost dominates.
fn bench_faust_render(c: &mut Criterion) {
    for &(id, source) in &DSP_SOURCES {
        bench_faust_render_one(c, id, id, source, BLOCK_SIZE as u16);
    }
    bench_faust_render_one(c, "wahpro2", "wahpro2", WAH_PRO2, 1);
}

/// Benchmark a full engine rendering `BLOCKS_PER_ITER` blocks, either a deep
/// serial chain or parallel branches of the given effect names.
fn bench_faust_engine(c: &mut Criterion, id: &str, effect_names: &[&str], parallel: bool) {
    let names: Vec<&str> = effect_names.to_vec();
    c.bench_function(&format!("faust_engine_{id}"), |b| {
        b.iter_batched(
            || build_engine(&names, parallel),
            |mut engine| {
                let mut buf = [[0.0f32; 2]; BLOCK_SIZE];
                for _ in 0..BLOCKS_PER_ITER {
                    engine.render_block(&mut buf);
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

/// The effect names for an n-long chain, cycling through the five DSPs.
fn fx_names(n: usize) -> Vec<&'static str> {
    (0..n).map(|i| FX_NAMES[i % FX_NAMES.len()]).collect()
}

/// Engine-level benchmarks: deep serial chains and parallel branches.
fn bench_faust_engine_topologies(c: &mut Criterion) {
    bench_faust_engine(
        c,
        "deep_4",
        &["FSM WahPro2", "Jeskola Reverb 2", "Jeskola Filter 2", "Jeskola Freeverb"],
        false,
    );
    bench_faust_engine(c, "deep_8", &fx_names(8), false);
    bench_faust_engine(c, "parallel_4", &fx_names(4), true);
    bench_faust_engine(c, "parallel_8", &fx_names(8), true);
}

criterion_group!(
    benches,
    bench_faust_render,
    bench_faust_engine_topologies,
);
criterion_main!(benches);
