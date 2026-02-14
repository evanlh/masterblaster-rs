//! Runtime evaluator for `ModEnvelope`.

use mb_ir::{interpolate, ModEnvelope};

/// Runtime state for a playing envelope.
#[derive(Clone, Debug)]
pub struct EnvelopeState {
    /// Current segment index (the "from" breakpoint).
    segment: u16,
    /// Sub-beat units elapsed within the current segment.
    time_in_segment: u32,
    /// Current output value.
    value: f32,
    /// One-shot envelope reached its end.
    finished: bool,
    /// Holding at sustain point, waiting for gate-off.
    gate_held: bool,
    /// Whether a loop point was hit on the last advance (for Trigger mode).
    looped: bool,
}

impl EnvelopeState {
    /// Create a new state starting at the first breakpoint.
    pub fn new(envelope: &ModEnvelope) -> Self {
        let value = envelope.points.first().map_or(0.0, |p| p.value);
        Self { segment: 0, time_in_segment: 0, value, finished: false, gate_held: false, looped: false }
    }

    /// Current output value.
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Whether the envelope has finished (one-shot).
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Whether a loop point was crossed on the last advance (for Trigger mode).
    pub fn looped(&self) -> bool {
        self.looped
    }

    /// Release the sustain hold (gate-off).
    pub fn gate_off(&mut self) {
        self.gate_held = false;
    }

    /// Advance the envelope by `delta` sub-beat units.
    pub fn advance(&mut self, envelope: &ModEnvelope, delta: u32) {
        self.looped = false;
        if self.finished || self.gate_held || envelope.points.len() < 2 {
            return;
        }

        self.time_in_segment += delta;
        self.resolve(envelope);
    }

    /// Walk forward through breakpoints until time_in_segment is within
    /// the current segment, handling loop and sustain.
    fn resolve(&mut self, envelope: &ModEnvelope) {
        loop {
            let seg_idx = self.segment as usize;
            let next_idx = seg_idx + 1;
            if next_idx >= envelope.points.len() {
                self.finished = true;
                self.value = envelope.points[seg_idx].value;
                return;
            }

            let next = &envelope.points[next_idx];
            if next.dt == 0 || self.time_in_segment >= next.dt {
                // Crossed into the next breakpoint
                let overshoot = if next.dt > 0 { self.time_in_segment - next.dt } else { self.time_in_segment };
                self.segment += 1;
                self.time_in_segment = overshoot;
                self.value = next.value;

                // Check sustain
                if envelope.sustain_point == Some(self.segment) {
                    self.gate_held = true;
                    self.time_in_segment = 0;
                    return;
                }

                // Check loop
                if let Some(ref lr) = envelope.loop_range {
                    if self.segment >= lr.end {
                        self.segment = lr.start;
                        self.looped = true;
                        self.value = envelope.points[lr.start as usize].value;
                        // Continue resolving with remaining time
                        if self.time_in_segment == 0 {
                            return;
                        }
                        continue;
                    }
                }

                // Check end
                if (self.segment as usize) + 1 >= envelope.points.len() {
                    self.finished = true;
                    return;
                }

                // Still have overshoot — continue resolving
                if self.time_in_segment > 0 {
                    continue;
                }
                return;
            } else {
                // Within the current segment — interpolate
                let seg = &envelope.points[seg_idx];
                let t = self.time_in_segment as f32 / next.dt as f32;
                self.value = interpolate(seg.curve, seg.value, next.value, t);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::{CurveKind, ModBreakPoint, ModEnvelope};

    fn bp(dt: u32, value: f32, curve: CurveKind) -> ModBreakPoint {
        ModBreakPoint::new(dt, value, curve)
    }

    #[test]
    fn single_linear_segment() {
        let env = ModEnvelope::one_shot(&[
            bp(0, 0.0, CurveKind::Linear),
            bp(100, 10.0, CurveKind::Step),
        ]);
        let mut state = EnvelopeState::new(&env);
        assert_eq!(state.value(), 0.0);

        state.advance(&env, 50);
        assert!((state.value() - 5.0).abs() < 0.01);

        state.advance(&env, 50);
        assert!((state.value() - 10.0).abs() < 0.01);
        assert!(state.is_finished());
    }

    #[test]
    fn step_interpolation_holds() {
        let env = ModEnvelope::one_shot(&[
            bp(0, 5.0, CurveKind::Step),
            bp(100, 10.0, CurveKind::Step),
        ]);
        let mut state = EnvelopeState::new(&env);

        state.advance(&env, 50);
        assert_eq!(state.value(), 5.0); // still holding first value

        state.advance(&env, 50);
        assert_eq!(state.value(), 10.0); // reached next point
    }

    #[test]
    fn looping_envelope_cycles() {
        let env = ModEnvelope::looping(
            &[
                bp(0, 0.0, CurveKind::Step),
                bp(10, 1.0, CurveKind::Step),
                bp(10, 2.0, CurveKind::Step),
            ],
            0,
            2,
        );
        let mut state = EnvelopeState::new(&env);

        // Advance to point 1
        state.advance(&env, 10);
        assert_eq!(state.value(), 1.0);

        // Advance to point 2 → loop back to point 0
        state.advance(&env, 10);
        assert_eq!(state.value(), 0.0);
        assert!(state.looped());

        // Advance again to point 1
        state.advance(&env, 10);
        assert_eq!(state.value(), 1.0);
        assert!(!state.looped());

        // Not finished (looping)
        assert!(!state.is_finished());
    }

    #[test]
    fn sustain_holds_until_gate_off() {
        let mut env = ModEnvelope::one_shot(&[
            bp(0, 0.0, CurveKind::Linear),
            bp(10, 1.0, CurveKind::Linear),
            bp(0, 1.0, CurveKind::Linear),  // sustain hold
            bp(10, 0.0, CurveKind::Linear),  // release
        ]);
        env.sustain_point = Some(2);
        let mut state = EnvelopeState::new(&env);

        // Attack phase
        state.advance(&env, 10);
        assert!((state.value() - 1.0).abs() < 0.01);

        // Should be held at sustain
        state.advance(&env, 100);
        assert!((state.value() - 1.0).abs() < 0.01);
        assert!(!state.is_finished());

        // Gate off → release
        state.gate_off();
        state.advance(&env, 5);
        assert!(state.value() < 1.0);
        assert!(state.value() > 0.0);
    }

    #[test]
    fn trigger_mode_detects_loop() {
        let env = ModEnvelope::looping(
            &[
                bp(0, 0.0, CurveKind::Step),
                bp(30, 0.0, CurveKind::Step),
            ],
            0,
            1,
        );
        let mut state = EnvelopeState::new(&env);

        // Not yet looped
        state.advance(&env, 10);
        assert!(!state.looped());

        // Still not
        state.advance(&env, 10);
        assert!(!state.looped());

        // Loop at 30
        state.advance(&env, 10);
        assert!(state.looped());

        // Next advance, not looped yet
        state.advance(&env, 10);
        assert!(!state.looped());
    }

    #[test]
    fn empty_envelope_stays_at_zero() {
        let env = ModEnvelope::one_shot(&[]);
        let mut state = EnvelopeState::new(&env);
        assert_eq!(state.value(), 0.0);
        state.advance(&env, 100);
        assert_eq!(state.value(), 0.0);
    }

    #[test]
    fn one_point_envelope_holds_value() {
        let env = ModEnvelope::one_shot(&[bp(0, 42.0, CurveKind::Linear)]);
        let mut state = EnvelopeState::new(&env);
        assert_eq!(state.value(), 42.0);
        state.advance(&env, 100);
        assert_eq!(state.value(), 42.0);
    }

    #[test]
    fn multi_segment_walks_through() {
        let env = ModEnvelope::one_shot(&[
            bp(0, 0.0, CurveKind::Linear),
            bp(10, 10.0, CurveKind::Linear),
            bp(10, 20.0, CurveKind::Step),
        ]);
        let mut state = EnvelopeState::new(&env);

        state.advance(&env, 10);
        assert!((state.value() - 10.0).abs() < 0.01);

        state.advance(&env, 5);
        assert!((state.value() - 15.0).abs() < 0.01);

        state.advance(&env, 5);
        assert!((state.value() - 20.0).abs() < 0.01);
        assert!(state.is_finished());
    }

    #[test]
    fn large_overshoot_skips_segments() {
        let env = ModEnvelope::one_shot(&[
            bp(0, 0.0, CurveKind::Linear),
            bp(10, 10.0, CurveKind::Linear),
            bp(10, 20.0, CurveKind::Step),
        ]);
        let mut state = EnvelopeState::new(&env);

        // Jump past both segments in one advance
        state.advance(&env, 25);
        assert!((state.value() - 20.0).abs() < 0.01);
        assert!(state.is_finished());
    }
}
