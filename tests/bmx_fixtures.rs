//! Integration tests for BMX format loading.

use mb_formats::load_bmx;
use mb_ir::NodeType;
use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bmx")
}

fn load_fixture(name: &str) -> mb_ir::Song {
    let path = fixtures_dir().join(name);
    let data = fs::read(&path).unwrap();
    load_bmx(&data).unwrap()
}

#[test]
fn load_tribal_60_succeeds() {
    let path = fixtures_dir().join("tribal-60.bmx");
    let data = fs::read(&path).unwrap();
    assert!(load_bmx(&data).is_ok());
}

#[test]
fn tribal_60_has_machines() {
    let song = load_fixture("tribal-60.bmx");
    // Should have more than just the Master node
    assert!(song.graph.nodes.len() > 1);
}

#[test]
fn tribal_60_has_master() {
    let song = load_fixture("tribal-60.bmx");
    let has_master = song
        .graph
        .nodes
        .iter()
        .any(|n| matches!(n.node_type, NodeType::Master));
    assert!(has_master);
}

#[test]
fn tribal_60_has_connections() {
    let song = load_fixture("tribal-60.bmx");
    assert!(!song.graph.connections.is_empty());
}

#[test]
fn tribal_60_has_tracks() {
    let song = load_fixture("tribal-60.bmx");
    assert!(!song.tracks.is_empty());
}

#[test]
fn tribal_60_has_samples() {
    let song = load_fixture("tribal-60.bmx");
    // WAVT section should produce samples
    assert!(!song.samples.is_empty());
}

#[test]
fn tribal_60_has_tracker_channels() {
    let song = load_fixture("tribal-60.bmx");
    let tracker_nodes: Vec<_> = song.graph.nodes.iter()
        .filter(|n| matches!(n.node_type, NodeType::TrackerChannel { .. }))
        .collect();
    assert!(!tracker_nodes.is_empty(), "should have TrackerChannel nodes");
}

#[test]
fn tribal_60_has_channel_settings() {
    let song = load_fixture("tribal-60.bmx");
    assert!(!song.channels.is_empty(), "should have ChannelSettings");
}

#[test]
fn tribal_60_has_instruments() {
    let song = load_fixture("tribal-60.bmx");
    assert!(!song.instruments.is_empty(), "should have instruments");
}

#[test]
fn tribal_60_tracker_tracks_have_cell_data() {
    let song = load_fixture("tribal-60.bmx");
    // Find tracks targeting TrackerChannel nodes
    let has_notes = song.tracks.iter().any(|track| {
        let is_tracker = song.graph.node(track.target)
            .map_or(false, |n| matches!(n.node_type, NodeType::TrackerChannel { .. }));
        is_tracker && track.clips.iter().any(|clip| {
            clip.pattern().map_or(false, |pat| {
                pat.data.iter().any(|cell| !cell.is_empty())
            })
        })
    });
    assert!(has_notes, "tracker tracks should have cell data");
}

#[test]
fn load_all_bmx_fixtures() {
    for name in &["tribal-60.bmx", "acousticelectro-drumloop-100.bmx", "Insomnium - Skooled RMX.bmx"] {
        let path = fixtures_dir().join(name);
        let data = fs::read(&path).unwrap();
        let result = load_bmx(&data);
        assert!(result.is_ok(), "Failed to load {}: {:?}", name, result.err());
    }
}
