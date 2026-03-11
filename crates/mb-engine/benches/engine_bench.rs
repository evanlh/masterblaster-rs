//! Criterion benchmarks for the engine render loop.

use criterion::{criterion_group, criterion_main, Criterion};
use mb_engine::Engine;
use mb_ir::{
    build_tracks, Cell, Instrument, Note, OrderEntry, Pattern, Sample, SampleData, Song,
};

const SAMPLE_RATE: u32 = 44100;
const FRAMES_PER_CHUNK: usize = 4410; // 100ms
const CHUNKS: usize = 100;
const PATTERN_ROWS: u16 = 64;
const SINE_LEN: usize = 8000;

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
    let hash = (row as u32).wrapping_mul(31).wrapping_add(channel as u32).wrapping_mul(17);
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

/// Build a benchmark song with the given channel count and pattern rows.
fn build_bench_song(num_channels: u8, rows: u16) -> Song {
    let mut song = Song::with_channels("bench", num_channels);

    let mut sample = Sample::new("sine");
    sample.data = SampleData::Mono8(sine_wave(SINE_LEN));
    sample.default_volume = 64;
    sample.c4_speed = 8363;
    song.samples.push(sample);

    let mut inst = Instrument::new("sine inst");
    inst.set_single_sample(0);
    song.instruments.push(inst);

    let pattern = build_pattern(rows, num_channels);
    let order = vec![OrderEntry::Pattern(0)];

    let tracker_id = mb_ir::find_tracker_node(&song.graph);
    song.tracks.push(mb_ir::Track::new(tracker_id, 0, num_channels));

    build_tracks(&mut song, &[pattern], &order);
    song
}

/// Build a song with N passthrough nodes chained between AmigaFilter and Master.
fn build_bench_song_with_passthrough(
    num_channels: u8,
    rows: u16,
    num_passthrough: usize,
) -> Song {
    let mut song = build_bench_song(num_channels, rows);

    if num_passthrough == 0 {
        return song;
    }

    // Current graph: Tracker(2) → AmigaFilter(1) → Master(0)
    // We want: Tracker(2) → AmigaFilter(1) → Pass1 → Pass2 → ... → PassN → Master(0)
    // Remove the AmigaFilter→Master connection, insert passthrough chain.

    // Find the AmigaFilter→Master connection and remove it
    song.graph.connections.retain(|c| !(c.from == 1 && c.to == 0));

    // Add passthrough nodes chained: AmigaFilter → P1 → P2 → ... → PN → Master
    let mut prev_id = 1u16; // AmigaFilter
    for i in 0..num_passthrough {
        let name = format!("Passthrough {}", i);
        let node_id = song.graph.add_node(mb_ir::NodeType::Machine {
            machine_name: name,
            is_tracker: false,
        });
        song.graph.connect(prev_id, node_id);
        prev_id = node_id;
    }
    song.graph.connect(prev_id, 0); // last passthrough → Master

    song
}

/// Create an engine ready for benchmarking (scheduled and playing).
fn setup_engine(song: Song) -> Engine {
    let mut engine = Engine::new(song, SAMPLE_RATE);
    engine.schedule_song();
    engine.play();
    engine
}

fn bench_render_10_channels(c: &mut Criterion) {
    c.bench_function("render_10ch_64rows_100x100ms", |b| {
        b.iter_batched(
            || setup_engine(build_bench_song(10, PATTERN_ROWS)),
            |mut engine| {
                let mut buf = [[0.0f32; 2]; FRAMES_PER_CHUNK];
                for _ in 0..CHUNKS {
                    engine.render_frames_into(&mut buf);
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_render_20_channels(c: &mut Criterion) {
    c.bench_function("render_20ch_64rows_100x100ms", |b| {
        b.iter_batched(
            || setup_engine(build_bench_song(20, PATTERN_ROWS)),
            |mut engine| {
                let mut buf = [[0.0f32; 2]; FRAMES_PER_CHUNK];
                for _ in 0..CHUNKS {
                    engine.render_frames_into(&mut buf);
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn bench_render_10_channels_10_passthrough(c: &mut Criterion) {
    c.bench_function("render_10ch_10pass_64rows_100x100ms", |b| {
        b.iter_batched(
            || setup_engine(build_bench_song_with_passthrough(10, PATTERN_ROWS, 10)),
            |mut engine| {
                let mut buf = [[0.0f32; 2]; FRAMES_PER_CHUNK];
                for _ in 0..CHUNKS {
                    engine.render_frames_into(&mut buf);
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    bench_render_10_channels,
    bench_render_20_channels,
    bench_render_10_channels_10_passthrough,
);
criterion_main!(benches);
