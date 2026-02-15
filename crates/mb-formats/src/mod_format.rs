//! ProTracker MOD format parser.

use alloc::vec::Vec;
use mb_ir::{
    build_tracks, Cell, Effect, Instrument, Note, OrderEntry, Pattern, Sample, SampleData, Song,
    VolumeCommand,
};

use crate::FormatError;

/// Load a MOD file from bytes.
pub fn load_mod(data: &[u8]) -> Result<Song, FormatError> {
    if data.len() < 1084 {
        return Err(FormatError::UnexpectedEof);
    }

    // Detect format by checking signature at offset 1080
    let sig = &data[1080..1084];
    let num_channels = match sig {
        b"M.K." | b"M!K!" | b"FLT4" => 4,
        b"6CHN" => 6,
        b"8CHN" | b"OCTA" => 8,
        _ => {
            // Could be 15-sample format (no signature) or unknown
            // For now, assume 4 channels if no valid signature
            4
        }
    };

    // Parse header
    let title = parse_string(&data[0..20]);
    let mut song = Song::with_channels(&title, num_channels);
    song.rows_per_beat = 4; // MOD standard: 4 rows per beat

    // Parse sample headers (31 samples, starting at offset 20)
    for i in 0..31 {
        let header_offset = 20 + i * 30;
        let sample = parse_sample_header(&data[header_offset..header_offset + 30])?;

        // Track where sample data starts
        let sample_len = sample.len();
        if sample_len > 0 {
            // We'll load sample data after parsing patterns
        }

        song.samples.push(sample);

        // Create a simple instrument that maps to this sample
        let mut inst = Instrument::new(&format!("Sample {}", i + 1));
        inst.set_single_sample(i as u8);
        song.instruments.push(inst);
    }

    // Song length (number of positions in order list)
    let song_length = data[950] as usize;

    // Parse order list into local vec
    let mut order = Vec::new();
    for i in 0..song_length {
        let pattern_idx = data[952 + i];
        order.push(OrderEntry::Pattern(pattern_idx));
    }

    // Find highest pattern number to know how many patterns to load
    let max_pattern = data[952..952 + 128].iter().max().copied().unwrap_or(0) as usize;

    // Parse patterns into local vec
    let pattern_size = 64 * num_channels as usize * 4; // 64 rows, 4 bytes per cell
    let mut patterns = Vec::new();
    for pat_idx in 0..=max_pattern {
        let pat_offset = 1084 + pat_idx * pattern_size;
        if pat_offset + pattern_size > data.len() {
            break;
        }
        let pattern = parse_pattern(&data[pat_offset..pat_offset + pattern_size], num_channels)?;
        patterns.push(pattern);
    }

    // Load sample data
    let mut sample_offset: usize = 1084 + (max_pattern + 1) * pattern_size;
    for sample in &mut song.samples {
        let len = sample.len();
        if len > 0 && sample_offset + len <= data.len() {
            let sample_data: Vec<i8> = data[sample_offset..sample_offset + len]
                .iter()
                .map(|&b| b as i8)
                .collect();
            sample.data = SampleData::Mono8(sample_data);
            sample_offset += len;

            // Clamp loop bounds to actual sample length (common in real MOD files)
            if sample.loop_end > len as u32 {
                sample.loop_end = len as u32;
            }
        }
    }

    // Set initial tempo/speed (MOD defaults)
    song.initial_tempo = 125;
    song.initial_speed = 6;

    // Build per-track clips + sequences from parsed patterns/order
    build_tracks(&mut song, &patterns, &order);

    Ok(song)
}

/// Parse a null-terminated string from bytes.
fn parse_string(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).trim().to_string()
}

/// Parse a sample header (30 bytes).
fn parse_sample_header(data: &[u8]) -> Result<Sample, FormatError> {
    if data.len() < 30 {
        return Err(FormatError::UnexpectedEof);
    }

    let name = parse_string(&data[0..22]);
    let length = u16::from_be_bytes([data[22], data[23]]) as u32 * 2;
    let finetune = (data[24] & 0x0F) as i8;
    let finetune = if finetune > 7 { finetune - 16 } else { finetune };
    let volume = data[25].min(64);
    let loop_start = u16::from_be_bytes([data[26], data[27]]) as u32 * 2;
    let loop_length = u16::from_be_bytes([data[28], data[29]]) as u32 * 2;

    let mut sample = Sample::new(&name);
    sample.default_volume = volume;
    sample.c4_speed = 8363; // Standard Amiga frequency

    // Apply finetune to c4_speed
    if finetune != 0 {
        // Each finetune step is approximately 1/8 semitone
        let factor = 2.0_f32.powf(finetune as f32 / 96.0);
        sample.c4_speed = (sample.c4_speed as f32 * factor) as u32;
    }

    // Set up loop
    if loop_length > 2 {
        sample.loop_start = loop_start;
        sample.loop_end = loop_start + loop_length;
        sample.loop_type = mb_ir::LoopType::Forward;
    }

    // Placeholder for sample data (will be filled in later)
    sample.data = SampleData::Mono8(alloc::vec![0i8; length as usize]);

    Ok(sample)
}

/// Parse a pattern.
fn parse_pattern(data: &[u8], num_channels: u8) -> Result<Pattern, FormatError> {
    let mut pattern = Pattern::new(64, num_channels);

    for row in 0..64 {
        for ch in 0..num_channels {
            let offset = (row as usize * num_channels as usize + ch as usize) * 4;
            if offset + 4 > data.len() {
                return Err(FormatError::UnexpectedEof);
            }

            let cell = parse_cell(&data[offset..offset + 4]);
            *pattern.cell_mut(row, ch) = cell;
        }
    }

    Ok(pattern)
}

/// Parse a single pattern cell (4 bytes).
fn parse_cell(data: &[u8]) -> Cell {
    // MOD cell format:
    // Byte 0: Upper 4 bits of sample number, upper 4 bits of period
    // Byte 1: Lower 8 bits of period
    // Byte 2: Lower 4 bits of sample number, effect command
    // Byte 3: Effect parameter

    let sample_hi = data[0] & 0xF0;              // upper 4 bits → bits 4..7
    let period_hi = ((data[0] & 0x0F) as u16) << 8;
    let period_lo = data[1] as u16;
    let period = period_hi | period_lo;

    let sample_lo = (data[2] & 0xF0) >> 4;       // upper 4 bits → bits 0..3
    let sample = sample_hi | sample_lo;

    let effect_cmd = data[2] & 0x0F;
    let effect_param = data[3];

    // Convert period to note
    let note = period_to_note(period);

    // Parse effect
    let effect = parse_effect(effect_cmd, effect_param);

    Cell {
        note,
        instrument: sample,
        volume: VolumeCommand::None,
        effect,
    }
}

/// Convert Amiga period to MIDI note number.
fn period_to_note(period: u16) -> Note {
    if period == 0 {
        return Note::None;
    }

    // Standard Amiga period table for octaves 1-3
    // We'll use a simplified lookup
    const PERIODS: [u16; 36] = [
        856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, // Octave 1
        428, 404, 381, 360, 339, 320, 302, 285, 269, 254, 240, 226, // Octave 2
        214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113, // Octave 3
    ];

    // Find closest period
    let mut best_note = 0;
    let mut best_diff = u16::MAX;

    for (i, &p) in PERIODS.iter().enumerate() {
        let diff = (period as i32 - p as i32).unsigned_abs() as u16;
        if diff < best_diff {
            best_diff = diff;
            best_note = i;
        }
    }

    // Convert to MIDI note (C-1 = 24 in our system)
    Note::On((best_note + 36) as u8)
}

/// Parse a MOD effect command.
fn parse_effect(cmd: u8, param: u8) -> Effect {
    match cmd {
        0x0 if param != 0 => Effect::Arpeggio {
            x: (param >> 4) & 0x0F,
            y: param & 0x0F,
        },
        0x1 => Effect::PortaUp(param),
        0x2 => Effect::PortaDown(param),
        0x3 => Effect::TonePorta(param),
        0x4 => Effect::Vibrato {
            speed: (param >> 4) & 0x0F,
            depth: param & 0x0F,
        },
        0x5 => Effect::TonePortaVolSlide(param_to_slide(param)),
        0x6 => Effect::VibratoVolSlide(param_to_slide(param)),
        0x7 => Effect::Tremolo {
            speed: (param >> 4) & 0x0F,
            depth: param & 0x0F,
        },
        0x8 => Effect::SetPan(param),
        0x9 => Effect::SampleOffset(param),
        0xA => Effect::VolumeSlide(param_to_slide(param)),
        0xB => Effect::PositionJump(param),
        0xC => Effect::SetVolume(param.min(64)),
        0xD => Effect::PatternBreak(((param >> 4) * 10 + (param & 0x0F)).min(63)),
        0xE => parse_extended_effect(param),
        0xF => {
            if param < 32 {
                Effect::SetSpeed(param)
            } else {
                Effect::SetTempo(param)
            }
        }
        _ => Effect::None,
    }
}

/// Parse extended effect (Exx).
fn parse_extended_effect(param: u8) -> Effect {
    let cmd = (param >> 4) & 0x0F;
    let val = param & 0x0F;

    match cmd {
        0x1 => Effect::FinePortaUp(val),
        0x2 => Effect::FinePortaDown(val),
        0x4 => Effect::SetVibratoWaveform(val),
        0x5 => Effect::SetFinetune(if val > 7 { val as i8 - 16 } else { val as i8 }),
        0x6 => Effect::PatternLoop(val),
        0x7 => Effect::SetTremoloWaveform(val),
        0x8 => Effect::SetPanPosition(val),
        0x9 => Effect::RetriggerNote(val),
        0xA => Effect::FineVolumeSlideUp(val),
        0xB => Effect::FineVolumeSlideDown(val),
        0xC => Effect::NoteCut(val),
        0xD => Effect::NoteDelay(val),
        0xE => Effect::PatternDelay(val),
        _ => Effect::None,
    }
}

/// Convert volume slide parameter to signed value.
fn param_to_slide(param: u8) -> i8 {
    let up = (param >> 4) & 0x0F;
    let down = param & 0x0F;
    if up > 0 {
        up as i8
    } else {
        -(down as i8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_period_to_note() {
        // C-2 should be period 428
        assert_eq!(period_to_note(428), Note::On(48)); // C-4 in MIDI terms
        assert_eq!(period_to_note(0), Note::None);
    }
}
