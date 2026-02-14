//! Modulator types: routing envelopes to parameter targets.
//!
//! A `Modulator` pairs a `ModEnvelope` (the shape) with a `ModTarget`
//! (which parameter) and a `ModMode` (how to apply: add, multiply, set).

use crate::graph::NodeId;
use crate::mod_envelope::{CurveKind, ModBreakPoint, ModEnvelope};
use crate::musical_time::SUB_BEAT_UNIT;

// ── Core types ──────────────────────────────────────────────────────

/// Which channel parameter a modulator targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelParam {
    Volume,
    Period,
    Pan,
    SamplePosition,
}

/// Which global parameter a modulator targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalParam {
    Tempo,
    Speed,
}

/// What parameter a modulator targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModTarget {
    Channel { channel: u8, param: ChannelParam },
    Node { node: NodeId, param: u16 },
    Global(GlobalParam),
}

/// How the modulator output combines with the base value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModMode {
    /// output = base + modulator
    Add,
    /// output = base * modulator
    Multiply,
    /// output = modulator
    Set,
    /// Each loop point fires a discrete action
    Trigger,
}

/// A modulation source attached to a parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Modulator {
    pub source: ModEnvelope,
    pub target: ModTarget,
    pub mode: ModMode,
}

// ── Builder helpers ─────────────────────────────────────────────────

/// Compute sub-beats per tick from current speed and rows_per_beat.
pub fn sub_beats_per_tick(speed: u8, rows_per_beat: u8) -> u32 {
    let tpb = speed as u32 * rows_per_beat as u32;
    if tpb == 0 { return SUB_BEAT_UNIT; }
    SUB_BEAT_UNIT / tpb
}

/// Build a volume slide envelope (Set mode, linear ramp to boundary).
pub fn volume_slide_envelope(current: f32, rate: f32, spt: u32) -> ModEnvelope {
    let target = if rate > 0.0 { 64.0 } else { 0.0 };
    let diff = (target - current).abs();
    let dt_ticks = if rate.abs() < 1e-6 { 1 } else { (diff / rate.abs()).ceil() as u32 };
    let dt = dt_ticks * spt;
    ModEnvelope::one_shot(&[
        ModBreakPoint::new(0, current, CurveKind::Linear),
        ModBreakPoint::new(dt.max(1), target, CurveKind::Step),
    ])
}

/// Build a portamento envelope (Set mode, linear ramp toward period boundary).
pub fn porta_envelope(current: f32, rate: f32, min: f32, max: f32, spt: u32) -> ModEnvelope {
    let target = if rate < 0.0 { min } else { max };
    let diff = (target - current).abs();
    let dt_ticks = if rate.abs() < 1e-6 { 1 } else { (diff / rate.abs()).ceil() as u32 };
    let dt = dt_ticks * spt;
    ModEnvelope::one_shot(&[
        ModBreakPoint::new(0, current, CurveKind::Linear),
        ModBreakPoint::new(dt.max(1), target, CurveKind::Step),
    ])
}

/// Build a tone portamento envelope (Set mode, ramp toward target period).
pub fn tone_porta_envelope(current: f32, target: f32, speed: f32, spt: u32) -> ModEnvelope {
    let diff = (target - current).abs();
    let dt_ticks = if speed < 1e-6 { 1 } else { (diff / speed).ceil() as u32 };
    let dt = dt_ticks * spt;
    ModEnvelope::one_shot(&[
        ModBreakPoint::new(0, current, CurveKind::Linear),
        ModBreakPoint::new(dt.max(1), target, CurveKind::Step),
    ])
}

/// Build a vibrato envelope (Add mode, sine LFO on period).
///
/// `speed` is phase advance per tick (ProTracker units).
/// `depth` is period modulation amplitude.
pub fn add_mode_sine_envelope(speed: u8, depth: u8, spt: u32) -> ModEnvelope {
    // ProTracker vibrato: period = 64 / speed ticks per cycle
    // Quarter-cycle = 64 / speed / 4 ticks
    // But ProTracker phase is 0-63, speed is phase-advance per tick
    // Full cycle = 64 / speed ticks
    let quarter_ticks = if speed == 0 { 16 } else { 16u32 / speed as u32 };
    let quarter_dt = quarter_ticks.max(1) * spt;
    let d = depth as f32;
    ModEnvelope::looping(
        &[
            ModBreakPoint::new(0, 0.0, CurveKind::SineQuarter),
            ModBreakPoint::new(quarter_dt, d, CurveKind::SineQuarter),
            ModBreakPoint::new(quarter_dt, 0.0, CurveKind::SineQuarter),
            ModBreakPoint::new(quarter_dt, -d, CurveKind::SineQuarter),
            ModBreakPoint::new(quarter_dt, 0.0, CurveKind::Step),
        ],
        0,
        4,
    )
}

/// Build an arpeggio envelope (Add mode, step cycle on period).
///
/// `offsets` are pre-computed period offsets for each step.
pub fn arpeggio_envelope(offsets: [f32; 3], spt: u32) -> ModEnvelope {
    ModEnvelope::looping(
        &[
            ModBreakPoint::new(0, offsets[0], CurveKind::Step),
            ModBreakPoint::new(spt, offsets[1], CurveKind::Step),
            ModBreakPoint::new(spt, offsets[2], CurveKind::Step),
            ModBreakPoint::new(spt, offsets[0], CurveKind::Step),
        ],
        0,
        3,
    )
}

/// Build a retrigger envelope (Trigger mode, periodic loop).
pub fn retrigger_envelope(interval: u8, spt: u32) -> ModEnvelope {
    let dt = interval as u32 * spt;
    ModEnvelope::looping(
        &[
            ModBreakPoint::new(0, 0.0, CurveKind::Step),
            ModBreakPoint::new(dt.max(1), 0.0, CurveKind::Step),
        ],
        0,
        1,
    )
}

/// Build an ADSR envelope (Multiply mode, beat-relative timing).
pub fn adsr_envelope(
    attack_sub_beats: u32,
    decay_sub_beats: u32,
    sustain_level: f32,
    release_sub_beats: u32,
) -> ModEnvelope {
    let mut env = ModEnvelope::one_shot(&[
        ModBreakPoint::new(0, 0.0, CurveKind::Linear),
        ModBreakPoint::new(attack_sub_beats, 1.0, CurveKind::Exponential(0.3)),
        ModBreakPoint::new(decay_sub_beats, sustain_level, CurveKind::Exponential(-0.5)),
        ModBreakPoint::new(0, sustain_level, CurveKind::Linear),
        ModBreakPoint::new(release_sub_beats, 0.0, CurveKind::Exponential(-1.0)),
    ]);
    env.sustain_point = Some(3);
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPT: u32 = 30030; // default speed=6, rpb=4

    #[test]
    fn sub_beats_per_tick_default() {
        assert_eq!(sub_beats_per_tick(6, 4), 30030);
    }

    #[test]
    fn volume_slide_envelope_ramps_up() {
        let env = volume_slide_envelope(0.0, 2.0, SPT);
        assert_eq!(env.points.len(), 2);
        assert_eq!(env.points[0].value, 0.0);
        assert_eq!(env.points[1].value, 64.0);
        // 32 ticks to go 0→64 at rate 2
        assert_eq!(env.points[1].dt, 32 * SPT);
    }

    #[test]
    fn volume_slide_envelope_ramps_down() {
        let env = volume_slide_envelope(64.0, -4.0, SPT);
        assert_eq!(env.points[1].value, 0.0);
        assert_eq!(env.points[1].dt, 16 * SPT);
    }

    #[test]
    fn tone_porta_envelope_computes_duration() {
        let env = tone_porta_envelope(428.0, 214.0, 8.0, SPT);
        assert_eq!(env.points[0].value, 428.0);
        assert_eq!(env.points[1].value, 214.0);
        // ceil((428-214)/8) = 27 ticks
        assert_eq!(env.points[1].dt, 27 * SPT);
    }

    #[test]
    fn vibrato_envelope_is_looping() {
        let env = add_mode_sine_envelope(4, 8, SPT);
        assert_eq!(env.points.len(), 5);
        assert!(env.loop_range.is_some());
        assert_eq!(env.points[0].value, 0.0);
        assert_eq!(env.points[1].value, 8.0);
        assert_eq!(env.points[3].value, -8.0);
    }

    #[test]
    fn arpeggio_envelope_is_3_step_loop() {
        let env = arpeggio_envelope([0.0, -214.0, -315.0], SPT);
        assert_eq!(env.points.len(), 4);
        assert!(env.loop_range.is_some());
        let lr = env.loop_range.unwrap();
        assert_eq!(lr.start, 0);
        assert_eq!(lr.end, 3);
    }

    #[test]
    fn retrigger_envelope_loops() {
        let env = retrigger_envelope(3, SPT);
        assert_eq!(env.points.len(), 2);
        assert!(env.loop_range.is_some());
        assert_eq!(env.points[1].dt, 3 * SPT);
    }

    #[test]
    fn adsr_envelope_has_sustain_point() {
        let env = adsr_envelope(SUB_BEAT_UNIT / 2, SUB_BEAT_UNIT, 0.7, SUB_BEAT_UNIT * 3 / 2);
        assert_eq!(env.sustain_point, Some(3));
        assert_eq!(env.points.len(), 5);
    }
}
