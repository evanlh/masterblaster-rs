//! Piecewise curve type for modulation sources.
//!
//! `ModEnvelope` is the universal modulation primitive — it encodes LFOs,
//! ADSR envelopes, automation lanes, linear ramps, and step sequences
//! as breakpoints with interpolation curves and optional loop/sustain markers.

use arrayvec::ArrayVec;
use core::f32::consts::FRAC_PI_2;

/// Maximum breakpoints per envelope. Covers all tracker effects (max 5 for
/// vibrato/tremolo) and ADSR (5 points). Automation lanes that exceed this
/// would need a different representation.
pub const MAX_BREAKPOINTS: usize = 8;

/// A piecewise curve over time with optional loop and sustain control points.
#[derive(Clone, Debug, PartialEq)]
pub struct ModEnvelope {
    /// Breakpoints defining the curve.
    /// The first point's `dt` is ignored (it starts at t=0).
    pub points: ArrayVec<ModBreakPoint, MAX_BREAKPOINTS>,

    /// Loop range: when playback reaches `end`, jump back to `start`.
    /// Encodes LFOs (loop the whole envelope) and sustain loops.
    pub loop_range: Option<LoopRange>,

    /// Hold at this point index until gate-off, then continue.
    /// This is how ADSR sustain works.
    pub sustain_point: Option<u16>,
}

/// A breakpoint in a modulation envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModBreakPoint {
    /// Sub-beat units from previous point (0 for first point).
    pub dt: u32,
    /// Value at this point.
    pub value: f32,
    /// How to interpolate FROM this point TO the next.
    pub curve: CurveKind,
}

/// Loop range within an envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRange {
    /// Point index to loop back to.
    pub start: u16,
    /// Point index that triggers the loop.
    pub end: u16,
}

/// Interpolation curve between two breakpoints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurveKind {
    /// Hold this value until the next point (arpeggio, tremor).
    Step,
    /// Straight line to the next point.
    Linear,
    /// Sine quarter-wave interpolation (smooth LFO).
    SineQuarter,
    /// Exponential curve. 0.0 = linear, >0 = starts slow, <0 = starts fast.
    Exponential(f32),
}

/// Interpolate between two values using the given curve at position `t` (0.0..1.0).
pub fn interpolate(curve: CurveKind, from: f32, to: f32, t: f32) -> f32 {
    let factor = match curve {
        CurveKind::Step => 0.0,
        CurveKind::Linear => t,
        CurveKind::SineQuarter => libm::sinf(t * FRAC_PI_2),
        CurveKind::Exponential(k) => {
            if k.abs() < 1e-6 {
                t // degenerate to linear
            } else {
                (libm::expf(k * t) - 1.0) / (libm::expf(k) - 1.0)
            }
        }
    };
    from + (to - from) * factor
}

impl ModEnvelope {
    /// Create a one-shot envelope from a slice of breakpoints.
    pub fn one_shot(pts: &[ModBreakPoint]) -> Self {
        let mut points = ArrayVec::new();
        for p in pts { points.push(*p); }
        Self { points, loop_range: None, sustain_point: None }
    }

    /// Create a looping envelope from a slice of breakpoints.
    pub fn looping(pts: &[ModBreakPoint], loop_start: u16, loop_end: u16) -> Self {
        let mut points = ArrayVec::new();
        for p in pts { points.push(*p); }
        Self {
            points,
            loop_range: Some(LoopRange { start: loop_start, end: loop_end }),
            sustain_point: None,
        }
    }

    /// Number of breakpoints.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether the envelope has no breakpoints.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

impl ModBreakPoint {
    /// Create a new breakpoint.
    pub fn new(dt: u32, value: f32, curve: CurveKind) -> Self {
        Self { dt, value, curve }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_step_holds_value() {
        assert_eq!(interpolate(CurveKind::Step, 0.0, 10.0, 0.0), 0.0);
        assert_eq!(interpolate(CurveKind::Step, 0.0, 10.0, 0.5), 0.0);
        assert_eq!(interpolate(CurveKind::Step, 0.0, 10.0, 0.99), 0.0);
    }

    #[test]
    fn interpolate_linear_midpoint() {
        assert_eq!(interpolate(CurveKind::Linear, 0.0, 10.0, 0.0), 0.0);
        assert_eq!(interpolate(CurveKind::Linear, 0.0, 10.0, 0.5), 5.0);
        assert_eq!(interpolate(CurveKind::Linear, 0.0, 10.0, 1.0), 10.0);
    }

    #[test]
    fn interpolate_linear_negative_range() {
        assert_eq!(interpolate(CurveKind::Linear, 10.0, -10.0, 0.5), 0.0);
    }

    #[test]
    fn interpolate_sine_quarter_endpoints() {
        let v0 = interpolate(CurveKind::SineQuarter, 0.0, 10.0, 0.0);
        let v1 = interpolate(CurveKind::SineQuarter, 0.0, 10.0, 1.0);
        assert!((v0 - 0.0).abs() < 0.01);
        assert!((v1 - 10.0).abs() < 0.01);
    }

    #[test]
    fn interpolate_sine_quarter_midpoint_above_linear() {
        // Sine quarter at t=0.5 should be sin(π/4) ≈ 0.707, above linear 0.5
        let sine_mid = interpolate(CurveKind::SineQuarter, 0.0, 10.0, 0.5);
        let linear_mid = interpolate(CurveKind::Linear, 0.0, 10.0, 0.5);
        assert!(sine_mid > linear_mid);
    }

    #[test]
    fn interpolate_exponential_zero_is_linear() {
        let exp_mid = interpolate(CurveKind::Exponential(0.0), 0.0, 10.0, 0.5);
        let lin_mid = interpolate(CurveKind::Linear, 0.0, 10.0, 0.5);
        assert!((exp_mid - lin_mid).abs() < 0.01);
    }

    #[test]
    fn interpolate_exponential_positive_starts_slow() {
        // Positive k: starts slow, ends fast
        let mid = interpolate(CurveKind::Exponential(3.0), 0.0, 10.0, 0.5);
        assert!(mid < 5.0, "positive k should be below linear midpoint, got {}", mid);
    }

    #[test]
    fn interpolate_exponential_negative_starts_fast() {
        let mid = interpolate(CurveKind::Exponential(-3.0), 0.0, 10.0, 0.5);
        assert!(mid > 5.0, "negative k should be above linear midpoint, got {}", mid);
    }

    #[test]
    fn envelope_one_shot_construction() {
        let env = ModEnvelope::one_shot(&[
            ModBreakPoint::new(0, 0.0, CurveKind::Linear),
            ModBreakPoint::new(100, 1.0, CurveKind::Step),
        ]);
        assert_eq!(env.len(), 2);
        assert!(env.loop_range.is_none());
        assert!(env.sustain_point.is_none());
    }

    #[test]
    fn envelope_looping_construction() {
        let env = ModEnvelope::looping(
            &[
                ModBreakPoint::new(0, 0.0, CurveKind::SineQuarter),
                ModBreakPoint::new(100, 1.0, CurveKind::SineQuarter),
                ModBreakPoint::new(100, 0.0, CurveKind::Step),
            ],
            0,
            2,
        );
        assert_eq!(env.loop_range, Some(LoopRange { start: 0, end: 2 }));
    }
}
