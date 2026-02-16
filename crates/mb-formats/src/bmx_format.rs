//! Buzz BMX format parser.
//!
//! Parses BMX (Buzz Machine eXtended) files into the Song IR.
//! Reference: Buzztrax song-io-buzz.c and BMX wiki.

use alloc::string::String;
use alloc::vec::Vec;
use mb_ir::{
    AudioGraph, Cell, ChannelSettings, Clip, Connection, Instrument, LoopType,
    MusicalTime, NodeId, NodeType, Note, Parameter, Pattern, Sample, SampleData, SeqEntry, Song,
    Track, VolumeCommand,
};

use crate::FormatError;
use crate::effect_parser::parse_effect;

// ---------------------------------------------------------------------------
// BmxReader — cursor over a byte slice
// ---------------------------------------------------------------------------

struct BmxReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BmxReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    fn skip(&mut self, n: usize) -> Result<(), FormatError> {
        if self.pos + n > self.data.len() {
            return Err(FormatError::UnexpectedEof);
        }
        self.pos += n;
        Ok(())
    }

    fn read_u8(&mut self) -> Result<u8, FormatError> {
        if self.pos >= self.data.len() {
            return Err(FormatError::UnexpectedEof);
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn read_u16_le(&mut self) -> Result<u16, FormatError> {
        if self.pos + 2 > self.data.len() {
            return Err(FormatError::UnexpectedEof);
        }
        let v = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn read_u32_le(&mut self) -> Result<u32, FormatError> {
        if self.pos + 4 > self.data.len() {
            return Err(FormatError::UnexpectedEof);
        }
        let v = u32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    fn read_i32_le(&mut self) -> Result<i32, FormatError> {
        Ok(self.read_u32_le()? as i32)
    }

    fn read_f32_le(&mut self) -> Result<f32, FormatError> {
        if self.pos + 4 > self.data.len() {
            return Err(FormatError::UnexpectedEof);
        }
        let v = f32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    fn read_null_string(&mut self) -> Result<String, FormatError> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != 0 {
            self.pos += 1;
        }
        let s = String::from_utf8_lossy(&self.data[start..self.pos]).into_owned();
        if self.pos < self.data.len() {
            self.pos += 1; // skip null terminator
        }
        Ok(s)
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        if self.pos + n > self.data.len() {
            return Err(FormatError::UnexpectedEof);
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn read_var_uint(&mut self, width: u8) -> Result<u32, FormatError> {
        match width {
            1 => Ok(self.read_u8()? as u32),
            2 => Ok(self.read_u16_le()? as u32),
            4 => self.read_u32_le(),
            _ => Err(FormatError::UnsupportedVersion),
        }
    }

    /// Read a parameter value based on its type (1 byte or 2 bytes).
    fn read_param_value(&mut self, param_type: u8) -> Result<u16, FormatError> {
        match param_type {
            PT_WORD => Ok(self.read_u16_le()?),
            _ => Ok(self.read_u8()? as u16), // NOTE, SWITCH, BYTE, ENUM
        }
    }
}

// ---------------------------------------------------------------------------
// Parameter type constants
// ---------------------------------------------------------------------------

const PT_BYTE: u8 = 2;
const PT_WORD: u8 = 3;

// ---------------------------------------------------------------------------
// Intermediate types
// ---------------------------------------------------------------------------

struct SectionEntry {
    name: [u8; 4],
    offset: u32,
    #[allow(dead_code)]
    size: u32,
}

struct BmxParam {
    param_type: u8,
    name: String,
    min: i32,
    max: i32,
    #[allow(dead_code)]
    no_value: i32,
    #[allow(dead_code)]
    flags: i32,
    default: i32,
}

struct BmxParaDef {
    global_params: Vec<BmxParam>,
    track_params: Vec<BmxParam>,
}

impl BmxParaDef {
    /// Total bytes per row of global parameter state.
    fn global_byte_size(&self) -> usize {
        self.global_params.iter().map(|p| param_byte_size(p.param_type)).sum()
    }

    /// Total bytes per row of track parameter state.
    fn track_byte_size(&self) -> usize {
        self.track_params.iter().map(|p| param_byte_size(p.param_type)).sum()
    }
}

fn param_byte_size(param_type: u8) -> usize {
    if param_type == PT_WORD { 2 } else { 1 }
}

struct BmxMachine {
    name: String,
    #[allow(dead_code)]
    machine_type: u8,
    dll_name: Option<String>,
    node_id: NodeId,
    num_inputs: u16,
    is_tracker: bool,
    num_tracks: u16,
    channel_node_ids: Vec<NodeId>,
}

struct BmxPattern {
    name: String,
    ticks: u16,
    pattern: Option<Pattern>,
}

/// Returns true for DLL names that are tracker machines (cell-based).
fn is_tracker_dll(dll: &str) -> bool {
    matches!(dll, "Jeskola Tracker" | "Matilde Tracker" | "Matilde Tracker 2")
}

/// Convert a Buzz note byte to our Note type.
/// Buzz encoding: high nibble = octave, low nibble = note (1=C..12=B).
/// 0 = no note, 255 = note off.
fn buzz_note_to_note(buzz: u8) -> Note {
    match buzz {
        0 => Note::None,
        255 => Note::Off,
        _ => {
            let octave = buzz >> 4;
            let semi = (buzz & 0x0F).wrapping_sub(1);
            if semi < 12 {
                Note::On(octave * 12 + semi)
            } else {
                Note::None
            }
        }
    }
}

/// Convert a Buzz volume byte (0-254) to VolumeCommand. 0xFF = no change.
fn buzz_volume_to_cmd(vol: u8) -> VolumeCommand {
    if vol == 0xFF {
        VolumeCommand::None
    } else {
        // Buzz volume range is 0-0xFE, scale to 0-64
        let scaled = ((vol as u32) * 64 / 0xFE) as u8;
        VolumeCommand::Volume(scaled)
    }
}

/// Build a wave-index lookup: maps Buzz wave index → 1-based instrument number.
fn build_wave_lookup(bmx_waves: &[BmxWave]) -> Vec<(u16, u8)> {
    bmx_waves.iter().enumerate()
        .map(|(i, w)| (w.index, (i + 1) as u8))
        .collect()
}

/// Look up instrument number from Buzz wave index. 0 = no instrument.
fn wave_to_instrument(wave: u8, lookup: &[(u16, u8)]) -> u8 {
    if wave == 0 { return 0; }
    lookup.iter()
        .find(|(idx, _)| *idx == wave as u16)
        .map(|(_, inst)| *inst)
        .unwrap_or(0)
}

struct BmxWaveLevel {
    num_samples: u32,
    loop_start: u32,
    loop_end: u32,
    sample_rate: u32,
    #[allow(dead_code)]
    root_note: u8,
}

struct BmxWave {
    index: u16,
    name: String,
    volume: f32,
    flags: u8,
    levels: Vec<BmxWaveLevel>,
}

// ---------------------------------------------------------------------------
// Known machine parameter database (fallback when no PARA section)
// ---------------------------------------------------------------------------

/// Byte sizes for known Buzz machines: (global_bytes, track_bytes).
/// Derived from open-source buzzmachines and hex analysis.
fn known_machine_byte_sizes(dll_name: &str) -> Option<(usize, usize)> {
    match dll_name {
        "Jeskola Tracker" => Some((1, 5)),
        "Matilde Tracker" => Some((1, 5)),
        "Matilde Tracker 2" => Some((4, 7)),
        "Geonik's Compressor" => Some((7, 0)),
        "Geonik's Overdrive 2" => Some((5, 0)),
        "Jeskola Reverb 2" => Some((10, 0)),
        "Jeskola Filter 2" => Some((3, 0)),
        "Jeskola Delay" => Some((6, 0)),
        "Jeskola Racer" => Some((3, 0)),
        "Jeskola Mixer" => Some((1, 0)),
        "Jeskola Noise" => Some((2, 0)),
        "Jeskola Kick XP" => Some((9, 0)),
        _ => None,
    }
}

/// Build a synthetic BmxParaDef from known byte sizes (all BYTE params).
fn synthetic_para_def(global_bytes: usize, track_bytes: usize) -> BmxParaDef {
    let global_params = (0..global_bytes)
        .map(|i| BmxParam {
            param_type: PT_BYTE,
            name: alloc::format!("G{}", i),
            min: 0, max: 255, no_value: 0xFF, flags: 0, default: 0,
        })
        .collect();
    let track_params = (0..track_bytes)
        .map(|i| BmxParam {
            param_type: PT_BYTE,
            name: alloc::format!("T{}", i),
            min: 0, max: 255, no_value: 0xFF, flags: 0, default: 0,
        })
        .collect();
    BmxParaDef { global_params, track_params }
}

// ---------------------------------------------------------------------------
// Section directory
// ---------------------------------------------------------------------------

fn parse_header(r: &mut BmxReader) -> Result<Vec<SectionEntry>, FormatError> {
    let magic = r.read_bytes(4)?;
    if magic != b"Buzz" {
        return Err(FormatError::InvalidHeader);
    }
    let num_sections = r.read_u32_le()? as usize;
    let mut sections = Vec::with_capacity(num_sections);
    for _ in 0..num_sections {
        let name_bytes = r.read_bytes(4)?;
        let mut name = [0u8; 4];
        name.copy_from_slice(name_bytes);
        let offset = r.read_u32_le()?;
        let size = r.read_u32_le()?;
        sections.push(SectionEntry { name, offset, size });
    }
    Ok(sections)
}

fn find_section<'a>(sections: &'a [SectionEntry], name: &[u8; 4]) -> Option<&'a SectionEntry> {
    sections.iter().find(|s| &s.name == name)
}

// ---------------------------------------------------------------------------
// BVER
// ---------------------------------------------------------------------------

fn parse_bver(r: &mut BmxReader, entry: &SectionEntry) -> Result<String, FormatError> {
    r.seek(entry.offset as usize);
    let version = r.read_null_string()?;
    eprintln!("[BMX] Version: {}", version);
    Ok(version)
}

// ---------------------------------------------------------------------------
// PARA
// ---------------------------------------------------------------------------

fn parse_para(r: &mut BmxReader, entry: &SectionEntry) -> Result<Vec<BmxParaDef>, FormatError> {
    r.seek(entry.offset as usize);
    let num_machines = r.read_u32_le()? as usize;
    let mut defs = Vec::with_capacity(num_machines);
    for _ in 0..num_machines {
        let _name = r.read_null_string()?;
        let _long_name = r.read_null_string()?;
        let num_global = r.read_u32_le()? as usize;
        let num_track = r.read_u32_le()? as usize;
        let global_params = read_param_defs(r, num_global)?;
        let track_params = read_param_defs(r, num_track)?;
        defs.push(BmxParaDef { global_params, track_params });
    }
    eprintln!("[BMX] PARA: {} machine parameter definitions", defs.len());
    Ok(defs)
}

fn read_param_defs(r: &mut BmxReader, count: usize) -> Result<Vec<BmxParam>, FormatError> {
    let mut params = Vec::with_capacity(count);
    for _ in 0..count {
        let param_type = r.read_u8()?;
        let name = r.read_null_string()?;
        let min = r.read_i32_le()?;
        let max = r.read_i32_le()?;
        let no_value = r.read_i32_le()?;
        let flags = r.read_i32_le()?;
        let default = r.read_i32_le()?;
        params.push(BmxParam { param_type, name, min, max, no_value, flags, default });
    }
    Ok(params)
}

// ---------------------------------------------------------------------------
// Master fallback PARA (hardcoded when no PARA section)
// ---------------------------------------------------------------------------

fn master_para_def() -> BmxParaDef {
    BmxParaDef {
        global_params: alloc::vec![
            BmxParam { param_type: PT_WORD, name: String::from("Volume"), min: 0, max: 0x4000, no_value: 0xFFFF, flags: 2, default: 0 },
            BmxParam { param_type: PT_WORD, name: String::from("BPM"),    min: 0x10, max: 0x200, no_value: 0xFFFF, flags: 2, default: 126 },
            BmxParam { param_type: PT_BYTE, name: String::from("TPB"),    min: 1, max: 32, no_value: 0xFF, flags: 2, default: 4 },
        ],
        track_params: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// MACH
// ---------------------------------------------------------------------------

/// Resolve the PARA def for a single machine: use PARA section if available,
/// otherwise fall back to the known machine database.
fn resolve_para_def(machine_type: u8, dll_name: &Option<String>, para_defs: &Option<Vec<BmxParaDef>>, index: usize) -> BmxParaDef {
    if let Some(defs) = para_defs {
        if let Some(d) = defs.get(index) {
            return BmxParaDef {
                global_params: d.global_params.iter().map(|p| BmxParam {
                    param_type: p.param_type, name: p.name.clone(),
                    min: p.min, max: p.max, no_value: p.no_value,
                    flags: p.flags, default: p.default,
                }).collect(),
                track_params: d.track_params.iter().map(|p| BmxParam {
                    param_type: p.param_type, name: p.name.clone(),
                    min: p.min, max: p.max, no_value: p.no_value,
                    flags: p.flags, default: p.default,
                }).collect(),
            };
        }
    }
    // Fallback
    if machine_type == 0 {
        return master_para_def();
    }
    if let Some(dll) = dll_name {
        if let Some((gb, tb)) = known_machine_byte_sizes(dll) {
            return synthetic_para_def(gb, tb);
        }
        eprintln!("[BMX] WARNING: unknown machine \"{}\", assuming 0 params", dll);
    }
    BmxParaDef { global_params: Vec::new(), track_params: Vec::new() }
}

/// Parsed Master tempo settings.
struct MasterParams {
    bpm: u16,
    tpb: u8,
}

fn parse_mach(
    r: &mut BmxReader,
    entry: &SectionEntry,
    para_from_section: &Option<Vec<BmxParaDef>>,
    graph: &mut AudioGraph,
) -> Result<(Vec<BmxMachine>, Vec<BmxParaDef>, MasterParams), FormatError> {
    r.seek(entry.offset as usize);
    let num_machines = r.read_u16_le()? as usize;
    let mut machines = Vec::with_capacity(num_machines);
    let mut para_defs = Vec::with_capacity(num_machines);
    let mut next_channel_idx: u8 = 0;
    let mut master_bpm: u16 = 126;
    let mut master_tpb: u8 = 4;

    for i in 0..num_machines {
        let name = r.read_null_string()?;
        let machine_type = r.read_u8()?;
        let type_str = match machine_type {
            0 => "Master", 1 => "Generator", 2 => "Effect", _ => "Unknown",
        };

        let dll_name = if machine_type != 0 {
            Some(r.read_null_string()?)
        } else {
            None
        };

        let x = r.read_f32_le()?;
        let y = r.read_f32_le()?;

        // Skip machine-specific init data
        let data_size = r.read_u32_le()? as usize;
        r.skip(data_size)?;

        // Skip attributes
        let num_attrs = r.read_u16_le()? as usize;
        for _ in 0..num_attrs {
            let _attr_name = r.read_null_string()?;
            let _attr_val = r.read_u32_le()?;
        }

        // Resolve param defs and read/skip global param state
        let para = resolve_para_def(machine_type, &dll_name, para_from_section, i);
        if machine_type == 0 {
            // Master: read volume(u16) + bpm(u16) + tpb(u8)
            let remaining = para.global_byte_size();
            if remaining >= 5 {
                let _volume = r.read_u16_le()?;
                let bpm = r.read_u16_le()?;
                let tpb = r.read_u8()?;
                master_bpm = bpm;
                master_tpb = tpb;
                eprintln!("[BMX] Master params: bpm={}, tpb={}", bpm, tpb);
                r.skip(remaining - 5)?;
            } else {
                r.skip(remaining)?;
            }
        } else {
            r.skip(para.global_byte_size())?;
        }

        // Skip track param state
        let num_tracks = r.read_u16_le()? as usize;
        r.skip(num_tracks * para.track_byte_size())?;

        // Detect tracker machines
        let is_tracker = dll_name.as_deref().map_or(false, is_tracker_dll);

        // Create graph node(s)
        let (node_id, channel_node_ids) = if machine_type == 0 {
            (0, Vec::new())
        } else if is_tracker {
            // Create one TrackerChannel node per track
            // Channel index is sequential (0-based), NOT the graph node ID
            let mut ids = Vec::with_capacity(num_tracks);
            for t in 0..num_tracks {
                let ch_idx = next_channel_idx + t as u8;
                ids.push(graph.add_node(NodeType::TrackerChannel { index: ch_idx }));
            }
            next_channel_idx += num_tracks as u8;
            let primary = ids.first().copied().unwrap_or(0);
            (primary, ids)
        } else {
            let id = graph.add_node(NodeType::BuzzMachine { machine_name: name.clone() });
            // Add IR parameters to non-tracker graph nodes
            if let Some(node) = graph.node_mut(id) {
                for (j, p) in para.global_params.iter().enumerate() {
                    node.parameters.push(Parameter::new(
                        j as u16, &p.name, p.min, p.max, p.default,
                    ));
                }
            }
            (id, Vec::new())
        };

        eprintln!(
            "[BMX] Machine {}: \"{}\" type={} dll={} pos=({:.0},{:.0}){}",
            i, name, type_str, dll_name.as_deref().unwrap_or("(none)"), x, y,
            if is_tracker { alloc::format!(" [tracker, {} tracks]", num_tracks) } else { String::new() }
        );

        machines.push(BmxMachine {
            name, machine_type, dll_name, node_id, num_inputs: 0,
            is_tracker, num_tracks: num_tracks as u16, channel_node_ids,
        });
        para_defs.push(para);
    }

    eprintln!("[BMX] MACH: {} machines", machines.len());
    Ok((machines, para_defs, MasterParams { bpm: master_bpm, tpb: master_tpb }))
}

// ---------------------------------------------------------------------------
// CONN
// ---------------------------------------------------------------------------

fn parse_conn(
    r: &mut BmxReader,
    entry: &SectionEntry,
    machines: &mut [BmxMachine],
    graph: &mut AudioGraph,
) -> Result<(), FormatError> {
    r.seek(entry.offset as usize);
    let num_wires = r.read_u16_le()? as usize;

    for _ in 0..num_wires {
        let src_idx = r.read_u16_le()? as usize;
        let dst_idx = r.read_u16_le()? as usize;
        let amp = r.read_u16_le()?;
        let pan = r.read_u16_le()?;

        if src_idx < machines.len() && dst_idx < machines.len() {
            let gain = amplitude_to_gain(amp);
            let to_id = machines[dst_idx].node_id;

            if machines[src_idx].is_tracker {
                // Connect each TrackerChannel to the destination
                for &ch_id in &machines[src_idx].channel_node_ids {
                    graph.connections.push(Connection {
                        from: ch_id, to: to_id,
                        from_channel: 0, to_channel: 0, gain,
                    });
                }
            } else {
                graph.connections.push(Connection {
                    from: machines[src_idx].node_id, to: to_id,
                    from_channel: 0, to_channel: 0, gain,
                });
            }

            machines[dst_idx].num_inputs += 1;

            eprintln!(
                "[BMX] Wire: {} -> {} amp=0x{:04X} pan=0x{:04X}",
                machines[src_idx].name, machines[dst_idx].name, amp, pan
            );
        }
    }

    eprintln!("[BMX] CONN: {} wires", num_wires);
    Ok(())
}

/// Convert Buzz amplitude (0..0x4000) to gain in fixed-point dB.
fn amplitude_to_gain(amp: u16) -> i16 {
    if amp == 0 { return i16::MIN; }
    let ratio = amp as f32 / 0x4000 as f32;
    (ratio * 100.0 - 100.0) as i16
}

// ---------------------------------------------------------------------------
// PATT
// ---------------------------------------------------------------------------

fn parse_patt(
    r: &mut BmxReader,
    entry: &SectionEntry,
    machines: &[BmxMachine],
    para_defs: &[BmxParaDef],
    wave_lookup: &[(u16, u8)],
) -> Result<Vec<Vec<BmxPattern>>, FormatError> {
    r.seek(entry.offset as usize);
    let empty = BmxParaDef { global_params: Vec::new(), track_params: Vec::new() };
    let mut all_patterns: Vec<Vec<BmxPattern>> = Vec::with_capacity(machines.len());

    for (mi, mach) in machines.iter().enumerate() {
        let num_patterns = r.read_u16_le()? as usize;
        let num_tracks = r.read_u16_le()? as usize;
        let para = para_defs.get(mi).unwrap_or(&empty);
        let track_bytes = para.track_byte_size();
        let mut patterns = Vec::with_capacity(num_patterns);

        for _ in 0..num_patterns {
            let name = r.read_null_string()?;
            let num_ticks = r.read_u16_le()?;

            // Skip wire parameters: per input × (u16 src + num_ticks × (u16 amp + u16 pan))
            for _ in 0..mach.num_inputs {
                let _src_idx = r.read_u16_le()?;
                r.skip(num_ticks as usize * 4)?;
            }

            // Skip global parameters: num_ticks × global_byte_size
            r.skip(num_ticks as usize * para.global_byte_size())?;

            let pattern = if mach.is_tracker && track_bytes >= 5 {
                Some(read_tracker_pattern(r, num_ticks, num_tracks, track_bytes, wave_lookup)?)
            } else {
                // Skip track parameters for non-tracker machines
                r.skip(num_tracks * num_ticks as usize * track_bytes)?;
                None
            };

            patterns.push(BmxPattern { name, ticks: num_ticks, pattern });
        }

        if !patterns.is_empty() {
            eprintln!(
                "[BMX] PATT: machine \"{}\" has {} patterns{}",
                mach.name, patterns.len(),
                if mach.is_tracker { " [tracker cells]" } else { "" }
            );
        }

        all_patterns.push(patterns);
    }

    Ok(all_patterns)
}

/// Read tracker pattern cell data from track parameters.
/// Layout per tick per track: Note(u8), Wave(u8), Vol(u8), Effect(u8), EffectArg(u8)
/// Matilde Tracker 2 adds: Effect2(u8), EffectArg2(u8) (7 bytes total, second effect ignored).
fn read_tracker_pattern(
    r: &mut BmxReader,
    num_ticks: u16,
    num_tracks: usize,
    track_bytes: usize,
    wave_lookup: &[(u16, u8)],
) -> Result<Pattern, FormatError> {
    let mut pattern = Pattern::new(num_ticks, num_tracks as u8);
    let extra_bytes = track_bytes.saturating_sub(5);

    for track in 0..num_tracks {
        for tick in 0..num_ticks {
            let note_byte = r.read_u8()?;
            let wave_byte = r.read_u8()?;
            let vol_byte = r.read_u8()?;
            let effect_cmd = r.read_u8()?;
            let effect_arg = r.read_u8()?;
            // Skip extra bytes (e.g. Matilde Tracker 2's second effect column)
            r.skip(extra_bytes)?;

            let cell = Cell {
                note: buzz_note_to_note(note_byte),
                instrument: wave_to_instrument(wave_byte, wave_lookup),
                volume: buzz_volume_to_cmd(vol_byte),
                effect: parse_effect(effect_cmd, effect_arg),
            };

            if !cell.is_empty() {
                *pattern.cell_mut(tick, track as u8) = cell;
            }
        }
    }

    Ok(pattern)
}

// ---------------------------------------------------------------------------
// SEQU
// ---------------------------------------------------------------------------

fn parse_sequ(
    r: &mut BmxReader,
    entry: &SectionEntry,
    machines: &[BmxMachine],
    all_patterns: &[Vec<BmxPattern>],
    rows_per_beat: u8,
) -> Result<Vec<Track>, FormatError> {
    r.seek(entry.offset as usize);
    let end_of_song = r.read_u32_le()?;
    let loop_start = r.read_u32_le()?;
    let loop_end = r.read_u32_le()?;
    let num_sequences = r.read_u16_le()? as usize;

    eprintln!(
        "[BMX] SEQU: end={} loop={}..{} sequences={}",
        end_of_song, loop_start, loop_end, num_sequences
    );

    let rpb = rows_per_beat as u32;
    let mut tracks = Vec::with_capacity(num_sequences);

    for _ in 0..num_sequences {
        let machine_idx = r.read_u16_le()? as usize;
        let num_events = r.read_u32_le()? as usize;

        let (bpep, bpe) = if num_events > 0 {
            (r.read_u8()?, r.read_u8()?)
        } else {
            (0, 0)
        };

        let mach = machines.get(machine_idx);
        let mach_name = mach.map_or("?", |m| m.name.as_str());
        let is_tracker = mach.map_or(false, |m| m.is_tracker);

        // Parse sequence events
        let mut seq_entries = Vec::new();
        for _ in 0..num_events {
            let position = r.read_var_uint(bpep)?;
            let raw_event = r.read_var_uint(bpe)?;
            let event_id = extract_event_id(raw_event, bpe);

            if event_id >= 16 {
                let pat_idx = (event_id - 16) as u16;
                let start = MusicalTime::zero().add_rows(position, rpb);
                seq_entries.push(SeqEntry { start, clip_idx: pat_idx });
            }
        }

        if is_tracker && mach.is_some() {
            let m = mach.unwrap();
            let pats = all_patterns.get(machine_idx);
            // Create one Track per channel (TrackerChannel)
            for (ch_i, &ch_node_id) in m.channel_node_ids.iter().enumerate() {
                let track_name = alloc::format!("{} Ch{}", mach_name, ch_i + 1);
                let mut track = Track::new(ch_node_id, &track_name);
                track.group = Some(0);

                // Extract single-column clips from multi-channel patterns
                if let Some(pats) = pats {
                    for bp in pats {
                        let clip = match &bp.pattern {
                            Some(pat) => extract_single_column(pat, ch_i as u8),
                            None => Pattern::new(bp.ticks, 1),
                        };
                        track.clips.push(Clip::Pattern(clip));
                    }
                }

                track.sequence = seq_entries.clone();
                tracks.push(track);
            }
        } else {
            let node_id = mach.map_or(0, |m| m.node_id);
            let mut track = Track::new(node_id, mach_name);
            track.group = Some(0);

            // Add empty clips from this machine's pattern pool
            if let Some(pats) = all_patterns.get(machine_idx) {
                for bp in pats {
                    track.clips.push(Clip::Pattern(Pattern::new(bp.ticks, 1)));
                }
            }

            track.sequence = seq_entries;
            tracks.push(track);
        }

        if num_events > 0 {
            eprintln!(
                "[BMX] Sequence for \"{}\": {} events, {} pattern refs{}",
                mach_name, num_events,
                tracks.last().map_or(0, |t| t.sequence.len()),
                if is_tracker { " [per-channel]" } else { "" }
            );
        }
    }

    Ok(tracks)
}

/// Extract a single channel column from a multi-channel pattern.
fn extract_single_column(pattern: &Pattern, channel: u8) -> Pattern {
    let mut single = Pattern::new(pattern.rows, 1);
    single.rows_per_beat = pattern.rows_per_beat;
    for row in 0..pattern.rows {
        if channel < pattern.channels {
            *single.cell_mut(row, 0) = *pattern.cell(row, channel);
        }
    }
    single
}

fn extract_event_id(raw: u32, bpe: u8) -> u32 {
    match bpe {
        1 => raw & 0x7F,
        2 => raw & 0x7FFF,
        4 => raw & 0x7FFF_FFFF,
        _ => raw,
    }
}

// ---------------------------------------------------------------------------
// WAVT
// ---------------------------------------------------------------------------

fn parse_wavt(
    r: &mut BmxReader,
    entry: &SectionEntry,
) -> Result<Vec<BmxWave>, FormatError> {
    r.seek(entry.offset as usize);
    let num_waves = r.read_u16_le()? as usize;
    let mut waves = Vec::with_capacity(num_waves);

    for _ in 0..num_waves {
        let index = r.read_u16_le()?;
        let file_name = r.read_null_string()?;
        let name = r.read_null_string()?;
        let volume = r.read_f32_le()?;
        let flags = r.read_u8()?;

        let loop_enabled = flags & 0x01 != 0;
        let is_stereo = flags & 0x08 != 0;
        let bidi_loop = flags & 0x10 != 0;
        let has_envelopes = flags & 0x80 != 0;

        eprintln!(
            "[BMX] Wave {}: \"{}\" file=\"{}\" vol={:.2} loop={} stereo={} bidi={}",
            index, name, file_name, volume, loop_enabled, is_stereo, bidi_loop
        );

        if has_envelopes {
            skip_envelopes(r)?;
        }

        let num_levels = r.read_u8()? as usize;
        let mut levels = Vec::with_capacity(num_levels);
        for _ in 0..num_levels {
            let num_samples = r.read_u32_le()?;
            let loop_start = r.read_u32_le()?;
            let loop_end = r.read_u32_le()?;
            let sample_rate = r.read_u32_le()?;
            let root_note = r.read_u8()?;

            eprintln!(
                "  Level: {} samples, rate={} Hz, root={}",
                num_samples, sample_rate, root_note
            );

            levels.push(BmxWaveLevel {
                num_samples, loop_start, loop_end, sample_rate, root_note,
            });
        }

        waves.push(BmxWave { index, name, volume, flags, levels });
    }

    eprintln!("[BMX] WAVT: {} waves", waves.len());
    Ok(waves)
}

fn skip_envelopes(r: &mut BmxReader) -> Result<(), FormatError> {
    let num_envelopes = r.read_u16_le()? as usize;
    for _ in 0..num_envelopes {
        r.skip(10)?; // envelope header
        let raw_num_points = r.read_u16_le()?;
        let num_points = (raw_num_points & 0x7FFF) as usize;
        r.skip(num_points * 5)?; // each point: u16 x + u16 y + u8 flags
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CWAV / WAVE — audio sample data
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Buzz wave decompression (format=1)
// ---------------------------------------------------------------------------

/// Bit-stream reader for Buzz compressed wave data.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], start: usize) -> Self {
        Self { data, pos: start, bit: 0 }
    }

    /// Read `amount` bits (1..32) from the stream, LSB first.
    fn read_bits(&mut self, mut amount: u32) -> Result<u32, FormatError> {
        let mut result: u32 = 0;
        let mut shift: u32 = 0;
        while amount > 0 {
            if self.pos >= self.data.len() {
                return Err(FormatError::UnexpectedEof);
            }
            let avail = 8 - self.bit as u32;
            let take = amount.min(avail);
            let mask = (1u32 << take) - 1;
            let val = ((self.data[self.pos] >> self.bit) as u32) & mask;
            result |= val << shift;
            self.bit += take as u8;
            if self.bit >= 8 {
                self.bit = 0;
                self.pos += 1;
            }
            shift += take;
            amount -= take;
        }
        Ok(result)
    }

    /// Count consecutive zero bits (for variable-length prefix coding).
    fn count_zero_bits(&mut self) -> Result<u32, FormatError> {
        let mut count: u32 = 0;
        loop {
            let bit = self.read_bits(1)?;
            if bit != 0 { return Ok(count); }
            count += 1;
        }
    }

    /// Current byte position (for seek after decompression).
    fn byte_pos(&self) -> usize {
        if self.bit > 0 { self.pos + 1 } else { self.pos }
    }
}

/// Per-channel decompression state.
#[derive(Default)]
struct DecompState {
    sum1: i16,
    sum2: i16,
    result: i16,
}

/// Decode a signed value from the variable-length unsigned encoding.
/// Uses zigzag decoding: even → positive, odd → negative.
fn decode_signed(val: u32) -> i16 {
    if val & 1 == 0 {
        (val >> 1) as i16
    } else {
        (((val.wrapping_add(1)) >> 1) as i16).wrapping_neg()
    }
}

/// Decompress one block of samples for one channel.
fn decompress_block(
    br: &mut BitReader,
    state: &mut DecompState,
    out: &mut [i16],
) -> Result<(), FormatError> {
    let switch = br.read_bits(2)?;
    let bits = br.read_bits(4)?;

    for sample in out.iter_mut() {
        let val = br.read_bits(bits)?;
        let zeros = br.count_zero_bits()?;
        let combined = (zeros << bits) | val;
        let delta = decode_signed(combined);

        match switch {
            0 => {
                state.sum2 = delta.wrapping_sub(state.result).wrapping_sub(state.sum1);
                state.sum1 = delta.wrapping_sub(state.result);
                state.result = delta;
            }
            1 => {
                state.sum2 = delta.wrapping_sub(state.sum1);
                state.sum1 = delta;
                state.result = state.result.wrapping_add(delta);
            }
            2 => {
                state.sum2 = delta;
                state.sum1 = state.sum1.wrapping_add(delta);
                state.result = state.result.wrapping_add(state.sum1);
            }
            _ => {
                state.sum2 = state.sum2.wrapping_add(delta);
                state.sum1 = state.sum1.wrapping_add(state.sum2);
                state.result = state.result.wrapping_add(state.sum1);
            }
        }
        *sample = state.result;
    }
    Ok(())
}

/// Decompress a full Buzz compressed wave.
/// Returns decompressed i16 samples (interleaved for stereo).
fn decompress_wave(
    br: &mut BitReader,
    num_samples: usize,
    channels: usize,
) -> Result<Vec<i16>, FormatError> {
    // Skip leading zero bits
    let _leading = br.count_zero_bits()?;

    let shift = br.read_bits(4)? as usize;
    let block_size = 1usize << shift;
    let num_blocks = num_samples >> shift;
    let last_block_size = num_samples & (block_size - 1);
    let result_shift = br.read_bits(4)?;

    let sum_channels = if channels == 2 {
        br.read_bits(1)? != 0
    } else {
        false
    };

    let total = num_samples * channels;
    let mut output = vec![0i16; total];

    let mut states: Vec<DecompState> = (0..channels).map(|_| DecompState::default()).collect();

    let mut sample_offset = 0usize;
    let block_count = num_blocks + if last_block_size > 0 { 1 } else { 0 };

    for block_idx in 0..block_count {
        let bs = if block_idx == num_blocks { last_block_size } else { block_size };
        if bs == 0 { continue; }

        if channels == 1 {
            let start = sample_offset;
            let end = start + bs;
            decompress_block(br, &mut states[0], &mut output[start..end])?;
            apply_result_shift(&mut output[start..end], result_shift);
        } else {
            let mut ch0_buf = vec![0i16; bs];
            let mut ch1_buf = vec![0i16; bs];
            decompress_block(br, &mut states[0], &mut ch0_buf)?;
            decompress_block(br, &mut states[1], &mut ch1_buf)?;

            let start = sample_offset * 2;
            for i in 0..bs {
                let left = (ch0_buf[i] as i32) << result_shift;
                let right = if sum_channels {
                    ((ch1_buf[i] as i32) + (ch0_buf[i] as i32)) << result_shift
                } else {
                    (ch1_buf[i] as i32) << result_shift
                };
                output[start + i * 2] = left.clamp(-32768, 32767) as i16;
                output[start + i * 2 + 1] = right.clamp(-32768, 32767) as i16;
            }
        }
        sample_offset += bs;
    }

    Ok(output)
}

fn apply_result_shift(samples: &mut [i16], shift: u32) {
    if shift == 0 { return; }
    for s in samples.iter_mut() {
        let v = (*s as i32) << shift;
        *s = v.clamp(-32768, 32767) as i16;
    }
}

// ---------------------------------------------------------------------------
// CWAV / WAVE — sample data
// ---------------------------------------------------------------------------

fn parse_cwav(
    r: &mut BmxReader,
    entry: &SectionEntry,
    bmx_waves: &[BmxWave],
) -> Result<Vec<(u16, SampleData)>, FormatError> {
    r.seek(entry.offset as usize);
    let num_waves = r.read_u16_le()? as usize;
    let mut wave_data = Vec::with_capacity(num_waves);

    for _ in 0..num_waves {
        let index = r.read_u16_le()?;
        let format = r.read_u8()?;

        let bw = bmx_waves.iter().find(|w| w.index == index);
        let is_stereo = bw.map_or(false, |w| w.flags & 0x08 != 0);

        if format == 0 {
            // Raw uncompressed
            let _size_field = r.read_u32_le()?;

            if let Some(bw) = bw {
                for level in &bw.levels {
                    let channels: usize = if is_stereo { 2 } else { 1 };
                    let total_samples = level.num_samples as usize * channels;
                    let byte_count = total_samples * 2;

                    if r.pos + byte_count > r.data.len() {
                        eprintln!("[BMX] CWAV: truncated wave data for index {}", index);
                        break;
                    }

                    let data = read_i16_samples(r, total_samples)?;
                    let sample_data = if is_stereo {
                        deinterleave_stereo(&data)
                    } else {
                        SampleData::Mono16(data)
                    };
                    wave_data.push((index, sample_data));
                }
            }
        } else if format == 1 {
            // Buzz delta compression
            if let Some(bw) = bw {
                for level in &bw.levels {
                    let channels: usize = if is_stereo { 2 } else { 1 };
                    let mut br = BitReader::new(r.data, r.pos);
                    match decompress_wave(&mut br, level.num_samples as usize, channels) {
                        Ok(data) => {
                            r.pos = br.byte_pos();
                            let sample_data = if is_stereo {
                                deinterleave_stereo(&data)
                            } else {
                                SampleData::Mono16(data)
                            };
                            wave_data.push((index, sample_data));
                        }
                        Err(e) => {
                            eprintln!("[BMX] CWAV: decompression failed for wave {}: {:?}", index, e);
                            break;
                        }
                    }
                }
            }
        } else {
            eprintln!(
                "[BMX] CWAV: wave {} uses unknown format ({}), skipping remaining",
                index, format
            );
            break;
        }
    }

    eprintln!("[BMX] CWAV: loaded {} wave data entries", wave_data.len());
    Ok(wave_data)
}

fn read_i16_samples(r: &mut BmxReader, count: usize) -> Result<Vec<i16>, FormatError> {
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        samples.push(r.read_u16_le()? as i16);
    }
    Ok(samples)
}

fn deinterleave_stereo(interleaved: &[i16]) -> SampleData {
    let half = interleaved.len() / 2;
    let mut left = Vec::with_capacity(half);
    let mut right = Vec::with_capacity(half);
    for chunk in interleaved.chunks(2) {
        left.push(chunk[0]);
        if chunk.len() > 1 {
            right.push(chunk[1]);
        }
    }
    SampleData::Stereo16(left, right)
}

// ---------------------------------------------------------------------------
// Song IR assembly
// ---------------------------------------------------------------------------

/// Adjust sample rate for root_note offset.
///
/// Buzz root_note is in Buzz note format (octave<<4 | semitone, 1-based).
/// 0x41 = C-4 = our MIDI 48 (the c4_speed baseline). Other root notes
/// shift the effective playback rate.
fn root_note_adjusted_c4_speed(sample_rate: u32, root_note: u8) -> u32 {
    let midi = buzz_root_to_midi(root_note);
    if midi == 48 { return sample_rate; }
    let semitone_offset = midi as f32 - 48.0;
    (sample_rate as f32 * 2.0_f32.powf(semitone_offset / 12.0)) as u32
}

/// Convert Buzz root_note byte to MIDI note number.
/// Buzz: octave<<4 | semitone (1-based). 0x41 = C-4 = MIDI 48.
fn buzz_root_to_midi(root: u8) -> u8 {
    let octave = root >> 4;
    let semi = (root & 0x0F).wrapping_sub(1);
    if semi >= 12 { return 48; } // fallback to C-4
    octave * 12 + semi
}

fn build_samples(bmx_waves: &[BmxWave], wave_data: &[(u16, SampleData)]) -> Vec<Sample> {
    bmx_waves.iter().map(|bw| {
        let loop_type = match (bw.flags & 0x01 != 0, bw.flags & 0x10 != 0) {
            (false, _) => LoopType::None,
            (true, true) => LoopType::PingPong,
            (true, false) => LoopType::Forward,
        };

        let level = bw.levels.first();
        let data = wave_data
            .iter()
            .find(|(idx, _)| *idx == bw.index)
            .map(|(_, d)| d.clone())
            .unwrap_or_else(|| SampleData::Mono16(Vec::new()));

        let mut sample = Sample::new(&bw.name);
        sample.data = data;
        sample.loop_type = loop_type;
        sample.loop_start = level.map_or(0, |l| l.loop_start);
        sample.loop_end = level.map_or(0, |l| l.loop_end);
        sample.c4_speed = level.map_or(44100, |l| {
            root_note_adjusted_c4_speed(l.sample_rate, l.root_note)
        });
        sample.default_volume = (bw.volume * 64.0).clamp(0.0, 64.0) as u8;
        sample
    }).collect()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load a BMX file from bytes into a Song IR.
pub fn load_bmx(data: &[u8]) -> Result<Song, FormatError> {
    let mut r = BmxReader::new(data);

    // 1. Parse header and section directory
    let sections = parse_header(&mut r)?;
    eprintln!(
        "[BMX] {} sections: {}",
        sections.len(),
        sections.iter()
            .map(|s| String::from_utf8_lossy(&s.name).into_owned())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // 2. BVER (optional)
    if let Some(entry) = find_section(&sections, b"BVER") {
        let _version = parse_bver(&mut r, entry)?;
    }

    // 3. PARA (optional)
    let para_from_section = if let Some(entry) = find_section(&sections, b"PARA") {
        Some(parse_para(&mut r, entry)?)
    } else {
        None
    };

    // 4. MACH (required) — resolves PARA defs inline (fallback if no PARA section)
    let mach_entry = find_section(&sections, b"MACH").ok_or(FormatError::InvalidHeader)?;
    let mut graph = AudioGraph::with_master();
    let (mut machines, para_defs, master) = parse_mach(&mut r, mach_entry, &para_from_section, &mut graph)?;

    // 5. CONN (required)
    let conn_entry = find_section(&sections, b"CONN").ok_or(FormatError::InvalidHeader)?;
    parse_conn(&mut r, conn_entry, &mut machines, &mut graph)?;

    // 6. WAVT (optional, parsed before PATT so we have wave lookup for cell data)
    let bmx_waves = if let Some(entry) = find_section(&sections, b"WAVT") {
        parse_wavt(&mut r, entry)?
    } else {
        Vec::new()
    };
    let wave_lookup = build_wave_lookup(&bmx_waves);

    // 7. PATT (required)
    let patt_entry = find_section(&sections, b"PATT").ok_or(FormatError::InvalidHeader)?;
    let all_patterns = parse_patt(&mut r, patt_entry, &machines, &para_defs, &wave_lookup)?;

    // 8. SEQU (required)
    let rows_per_beat = master.tpb;
    let sequ_entry = find_section(&sections, b"SEQU").ok_or(FormatError::InvalidHeader)?;
    let tracks = parse_sequ(&mut r, sequ_entry, &machines, &all_patterns, rows_per_beat)?;

    // 9. CWAV / WAVE (optional)
    let wave_data = find_section(&sections, b"CWAV")
        .or_else(|| find_section(&sections, b"WAVE"))
        .map(|entry| parse_cwav(&mut r, entry, &bmx_waves))
        .transpose()?
        .unwrap_or_default();

    // Set up ChannelSettings for all TrackerChannel nodes
    let num_tracker_channels = machines.iter()
        .flat_map(|m| &m.channel_node_ids)
        .count();
    let channels: Vec<ChannelSettings> = (0..num_tracker_channels)
        .map(|i| ChannelSettings {
            // L-R-R-L panning pattern
            initial_pan: if i % 4 == 0 || i % 4 == 3 { -64 } else { 64 },
            initial_vol: 64,
            muted: false,
        })
        .collect();

    // Build instruments (one per wave, single-sample mapping)
    let instruments: Vec<Instrument> = bmx_waves.iter().enumerate()
        .map(|(i, w)| {
            let mut inst = Instrument::new(&w.name);
            inst.set_single_sample(i as u8);
            inst
        })
        .collect();

    // Assemble Song
    let mut song = Song::new("BMX Song");
    song.initial_speed = 1;
    let pt_tempo = (master.bpm as u32 * song.initial_speed as u32 * rows_per_beat as u32) / 24;
    song.initial_tempo = (pt_tempo.max(1).min(255)) as u8;
    song.rows_per_beat = rows_per_beat;
    song.graph = graph;
    song.tracks = tracks;
    song.channels = channels;
    song.instruments = instruments;
    song.samples = build_samples(&bmx_waves, &wave_data);

    let total = song.total_time();
    eprintln!(
        "[BMX] Song loaded: {} nodes, {} connections, {} tracks, {} samples, total={} beats",
        song.graph.nodes.len(),
        song.graph.connections.len(),
        song.tracks.len(),
        song.samples.len(),
        total.beat,
    );

    Ok(song)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_minimal_bmx() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Buzz");
        buf.extend_from_slice(&4u32.to_le_bytes());

        let dir_end = 8 + 4 * 12;

        // MACH: 1 machine (Master only)
        let mut mach_data = Vec::new();
        mach_data.extend_from_slice(&1u16.to_le_bytes());
        mach_data.extend_from_slice(b"Master\0");
        mach_data.push(0); // type=0
        mach_data.extend_from_slice(&0f32.to_le_bytes());
        mach_data.extend_from_slice(&0f32.to_le_bytes());
        mach_data.extend_from_slice(&0u32.to_le_bytes()); // data_size
        mach_data.extend_from_slice(&0u16.to_le_bytes()); // num_attrs
        // Master params: vol(u16) + bpm(u16) + tpb(u8)
        mach_data.extend_from_slice(&0x4000u16.to_le_bytes());
        mach_data.extend_from_slice(&126u16.to_le_bytes());
        mach_data.push(4);
        mach_data.extend_from_slice(&0u16.to_le_bytes()); // num_tracks

        let mut conn_data = Vec::new();
        conn_data.extend_from_slice(&0u16.to_le_bytes());

        let mut patt_data = Vec::new();
        patt_data.extend_from_slice(&0u16.to_le_bytes());
        patt_data.extend_from_slice(&0u16.to_le_bytes());

        let mut sequ_data = Vec::new();
        sequ_data.extend_from_slice(&0u32.to_le_bytes());
        sequ_data.extend_from_slice(&0u32.to_le_bytes());
        sequ_data.extend_from_slice(&0u32.to_le_bytes());
        sequ_data.extend_from_slice(&0u16.to_le_bytes());

        let mach_off = dir_end;
        let conn_off = mach_off + mach_data.len();
        let patt_off = conn_off + conn_data.len();
        let sequ_off = patt_off + patt_data.len();

        // Section directory
        for (name, off, data) in [
            (b"MACH", mach_off, &mach_data),
            (b"CONN", conn_off, &conn_data),
            (b"PATT", patt_off, &patt_data),
            (b"SEQU", sequ_off, &sequ_data),
        ] {
            buf.extend_from_slice(name);
            buf.extend_from_slice(&(off as u32).to_le_bytes());
            buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        }

        buf.extend_from_slice(&mach_data);
        buf.extend_from_slice(&conn_data);
        buf.extend_from_slice(&patt_data);
        buf.extend_from_slice(&sequ_data);
        buf
    }

    #[test]
    fn minimal_bmx_loads() {
        let data = make_minimal_bmx();
        let song = load_bmx(&data).unwrap();
        assert_eq!(song.graph.nodes.len(), 1);
        assert!(song.graph.connections.is_empty());
        assert!(song.tracks.is_empty());
    }

    #[test]
    fn invalid_magic_rejected() {
        assert!(load_bmx(b"NotBuzz\x00").is_err());
    }

    #[test]
    fn too_short_rejected() {
        assert!(load_bmx(b"Buz").is_err());
    }

    #[test]
    fn extract_event_id_masks_loop_flag() {
        assert_eq!(extract_event_id(0x90, 1), 0x10);
        assert_eq!(extract_event_id(0x8010, 2), 0x10);
        assert_eq!(extract_event_id(0x8000_0010, 4), 0x10);
    }

    #[test]
    fn amplitude_to_gain_unity() {
        assert_eq!(amplitude_to_gain(0x4000), 0);
    }

    #[test]
    fn amplitude_to_gain_half() {
        assert!(amplitude_to_gain(0x2000) < 0);
    }

    #[test]
    fn known_machine_lookup() {
        assert_eq!(known_machine_byte_sizes("Jeskola Tracker"), Some((1, 5)));
        assert_eq!(known_machine_byte_sizes("Unknown Machine"), None);
    }

    #[test]
    fn buzz_note_c4() {
        // Buzz C-4 = octave 4, note 1 (C) → MIDI 48
        assert_eq!(buzz_note_to_note(0x41), Note::On(48));
    }

    #[test]
    fn buzz_note_off() {
        assert_eq!(buzz_note_to_note(255), Note::Off);
    }

    #[test]
    fn buzz_note_none() {
        assert_eq!(buzz_note_to_note(0), Note::None);
    }

    #[test]
    fn buzz_note_a5() {
        // Buzz A-5 = octave 5, note 10 (A) → MIDI 69
        assert_eq!(buzz_note_to_note(0x5A), Note::On(69));
    }

    #[test]
    fn buzz_volume_none() {
        assert_eq!(buzz_volume_to_cmd(0xFF), VolumeCommand::None);
    }

    #[test]
    fn buzz_volume_max() {
        assert_eq!(buzz_volume_to_cmd(0xFE), VolumeCommand::Volume(64));
    }

    #[test]
    fn buzz_volume_zero() {
        assert_eq!(buzz_volume_to_cmd(0), VolumeCommand::Volume(0));
    }

    #[test]
    fn wave_lookup_maps_correctly() {
        let lookup = vec![(5u16, 1u8), (12, 2)];
        assert_eq!(wave_to_instrument(0, &lookup), 0);
        assert_eq!(wave_to_instrument(5, &lookup), 1);
        assert_eq!(wave_to_instrument(12, &lookup), 2);
        assert_eq!(wave_to_instrument(99, &lookup), 0);
    }

    #[test]
    fn is_tracker_dll_detects_trackers() {
        assert!(is_tracker_dll("Jeskola Tracker"));
        assert!(is_tracker_dll("Matilde Tracker"));
        assert!(is_tracker_dll("Matilde Tracker 2"));
        assert!(!is_tracker_dll("Jeskola Reverb 2"));
    }

    #[test]
    fn decode_signed_zigzag() {
        assert_eq!(decode_signed(0), 0);
        assert_eq!(decode_signed(1), -1);
        assert_eq!(decode_signed(2), 1);
        assert_eq!(decode_signed(3), -2);
        assert_eq!(decode_signed(4), 2);
        // Edge: u32::MAX → large negative, should not panic
        let _ = decode_signed(u32::MAX);
    }

    #[test]
    fn bit_reader_reads_across_bytes() {
        let data = [0b1010_0101, 0b1100_0011];
        let mut br = BitReader::new(&data, 0);
        assert_eq!(br.read_bits(4).unwrap(), 0b0101);
        assert_eq!(br.read_bits(4).unwrap(), 0b1010);
        assert_eq!(br.read_bits(8).unwrap(), 0b1100_0011);
    }

    #[test]
    fn bit_reader_count_zeros() {
        // 0b00000100 → 2 zeros then a 1
        let data = [0b0000_0100];
        let mut br = BitReader::new(&data, 0);
        assert_eq!(br.count_zero_bits().unwrap(), 2);
    }

    #[test]
    fn buzz_root_to_midi_c4() {
        // 0x41 = octave 4, semitone 1 (C) → MIDI 48
        assert_eq!(buzz_root_to_midi(0x41), 48);
    }

    #[test]
    fn buzz_root_to_midi_a4() {
        // 0x4A = octave 4, semitone 10 (A) → MIDI 57
        assert_eq!(buzz_root_to_midi(0x4A), 57);
    }

    #[test]
    fn root_note_c4_no_adjustment() {
        assert_eq!(root_note_adjusted_c4_speed(44100, 0x41), 44100);
    }

    #[test]
    fn root_note_c5_doubles_speed() {
        // C-5 = 0x51 = MIDI 60, 12 semitones above C-4
        let speed = root_note_adjusted_c4_speed(44100, 0x51);
        assert!((speed as f32 - 88200.0).abs() < 10.0);
    }
}
