//! Shared ProTracker-compatible effect parsing.
//!
//! Used by both the MOD and BMX format parsers. The effect encoding
//! is the same: command byte (0x0–0xF) + parameter byte.

use mb_ir::Effect;

/// Parse a ProTracker effect command.
pub fn parse_effect(cmd: u8, param: u8) -> Effect {
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
pub fn parse_extended_effect(param: u8) -> Effect {
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
pub fn param_to_slide(param: u8) -> i8 {
    let up = (param >> 4) & 0x0F;
    let down = param & 0x0F;
    if up > 0 {
        up as i8
    } else {
        -(down as i8)
    }
}
