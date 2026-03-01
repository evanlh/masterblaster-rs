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
fn tribal_60_has_tracker_machine() {
    let song = load_fixture("tribal-60.bmx");
    let tracker_nodes: Vec<_> = song.graph.nodes.iter()
        .filter(|n| matches!(&n.node_type, NodeType::BuzzMachine { machine_name } if machine_name == "Tracker"))
        .collect();
    assert!(!tracker_nodes.is_empty(), "should have Tracker machine nodes");
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
    // Check that tracker tracks (with num_channels > 0) have cell data
    let has_notes = song.tracks.iter().any(|track| {
        track.num_channels > 0 && track.clips.iter().any(|clip| {
            clip.pattern().map_or(false, |pat| {
                pat.data.iter().any(|cell| !cell.is_empty())
            })
        })
    });
    assert!(has_notes, "tracker tracks should have cell data");
}

#[test]
fn tribal_60_channels_match_tracker_tracks() {
    let song = load_fixture("tribal-60.bmx");
    // song.channels should have entries for each tracker channel
    assert!(!song.channels.is_empty(), "should have channel settings");
    // All tracker tracks should have num_channels > 0
    let total_tracker_channels: u8 = song.tracks.iter()
        .filter(|t| t.machine_node.is_some())
        .map(|t| t.num_channels)
        .sum();
    assert_eq!(song.channels.len(), total_tracker_channels as usize,
        "song.channels count should match total tracker channels");
}

#[test]
fn tribal_60_renders_without_panic() {
    let song = load_fixture("tribal-60.bmx");
    let ctrl = mb_master::Controller::new();
    // Use a mutable controller with the song loaded
    let mut ctrl = ctrl;
    ctrl.set_song(song);
    // Render a short segment — this would panic before the channel index fix
    let frames = ctrl.render_frames(44100, 44100); // 1 second
    assert!(!frames.is_empty(), "should render frames");
}

#[test]
fn tribal_60_has_correct_pt_tempo() {
    let song = load_fixture("tribal-60.bmx");
    // Buzz BPM 60, speed 1, rpb 8 → PT tempo = 60 * 1 * 8 / 24 = 20
    assert_eq!(song.initial_tempo, 20, "PT tempo should be 20 for BPM=60, rpb=8");
}

#[test]
fn acousticelectro_100_has_correct_pt_tempo() {
    let song = load_fixture("acousticelectro-drumloop-100.bmx");
    // Buzz BPM 100, speed 1, rpb 4 → PT tempo = 100 * 1 * 4 / 24 ≈ 16
    let expected = (100u32 * 1 * 4) / 24;
    assert_eq!(song.initial_tempo, expected as u8);
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
