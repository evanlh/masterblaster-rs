//! Channel state for tracker playback.

use mb_ir::{
    Effect, ModEnvelope, ModMode, Sample,
    add_mode_sine_envelope, arpeggio_envelope, note_cut_envelope, porta_envelope,
    retrigger_envelope, tone_porta_envelope, volume_slide_envelope,
};

use crate::envelope_state::EnvelopeState;
use crate::frequency::{clamp_period, note_to_period, period_to_increment, PERIOD_MAX, PERIOD_MIN};
use crate::frame::Frame;

/// An active envelope-based modulator on a channel parameter.
#[derive(Clone, Debug)]
pub struct ActiveMod {
    pub envelope: ModEnvelope,
    pub state: EnvelopeState,
    pub mode: ModMode,
}

impl ActiveMod {
    pub fn new(envelope: ModEnvelope, mode: ModMode) -> Self {
        let state = EnvelopeState::new(&envelope);
        Self { envelope, state, mode }
    }
}

/// Mixing state for a single tracker channel.
#[derive(Clone, Debug, Default)]
pub struct ChannelState {
    /// Current sample index
    pub sample_index: u8,
    /// Current position in sample (16.16 fixed-point)
    pub position: u32,
    /// Playback increment (16.16 fixed-point)
    pub increment: u32,
    /// Is the channel currently playing?
    pub playing: bool,

    // Base parameter values
    /// Current volume (0-64)
    pub volume: u8,
    /// Envelope tick position
    pub envelope_tick: u16,
    /// Current panning (-64 to +64)
    pub panning: i8,
    /// Current instrument
    pub instrument: u8,
    /// Current note
    pub note: u8,
    /// Loop direction for ping-pong (true = forward)
    pub loop_forward: bool,

    // Pitch state
    /// Current Amiga period (higher = lower pitch)
    pub period: u16,
    /// Sample's playback rate at C-4 (typically 8363 Hz)
    pub c4_speed: u32,
    /// Target period for tone portamento
    pub target_period: u16,
    /// Tone portamento speed (period units per tick)
    pub porta_speed: u8,

    // Effect memory (for tracker effect parameter persistence)
    /// Last vibrato speed
    pub vibrato_speed: u8,
    /// Last vibrato depth
    pub vibrato_depth: u8,
    /// Vibrato waveform (0=sine, 1=ramp, 2=square; bit 2=no retrig)
    pub vibrato_waveform: u8,
    /// Last tremolo speed
    pub tremolo_speed: u8,
    /// Last tremolo depth
    pub tremolo_depth: u8,
    /// Tremolo waveform (0=sine, 1=ramp, 2=square; bit 2=no retrig)
    pub tremolo_waveform: u8,

    // Envelope-based modulators (Add/Trigger mode)
    /// Period modulator (vibrato, arpeggio)
    pub period_mod: Option<ActiveMod>,
    /// Volume modulator (tremolo)
    pub volume_mod: Option<ActiveMod>,
    /// Trigger modulator (retrigger)
    pub trigger_mod: Option<ActiveMod>,

    // Computed per-tick modulation outputs
    /// Period offset from vibrato/arpeggio
    pub period_offset: i16,
    /// Volume offset from tremolo
    pub volume_offset: i8,
}

impl ChannelState {
    /// Create a new channel state.
    pub fn new() -> Self {
        Self {
            volume: 64,
            loop_forward: true,
            ..Default::default()
        }
    }

    /// Trigger a new note.
    pub fn trigger(&mut self, note: u8, instrument: u8, sample_index: u8) {
        self.note = note;
        self.instrument = instrument;
        self.sample_index = sample_index;
        self.position = 0;
        self.playing = true;
        self.envelope_tick = 0;
        self.loop_forward = true;
        self.period_offset = 0;
        self.volume_offset = 0;
        // Clear modulators (respect no-retrig waveform flag)
        if self.vibrato_waveform & 4 == 0 {
            self.period_mod = None;
        }
        if self.tremolo_waveform & 4 == 0 {
            self.volume_mod = None;
        }
        self.trigger_mod = None;
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Recompute the playback increment from the current period and c4_speed.
    /// Applies period_offset (from vibrato/arpeggio) without modifying the base period.
    pub fn update_increment(&mut self, sample_rate: u32) {
        if self.period > 0 {
            let effective = (self.period as i32 + self.period_offset as i32)
                .clamp(PERIOD_MIN as i32, PERIOD_MAX as i32) as u16;
            self.increment = period_to_increment(effective, self.c4_speed, sample_rate);
        }
    }

    /// Apply a row effect (first-tick / immediate).
    pub fn apply_row_effect(&mut self, effect: &Effect) {
        match effect {
            Effect::SetVolume(v) => self.volume = (*v).min(64),
            Effect::SetPan(p) => self.panning = (*p as i16 - 128).clamp(-64, 64) as i8,
            Effect::SampleOffset(o) => self.position = (*o as u32) << 24,
            Effect::FineVolumeSlideUp(v) => {
                self.volume = (self.volume as i16 + *v as i16).clamp(0, 64) as u8;
            }
            Effect::FineVolumeSlideDown(v) => {
                self.volume = (self.volume as i16 - *v as i16).clamp(0, 64) as u8;
            }
            Effect::FinePortaUp(v) => {
                self.period = clamp_period(self.period.saturating_sub(*v as u16));
            }
            Effect::FinePortaDown(v) => {
                self.period = clamp_period(self.period.saturating_add(*v as u16));
            }
            Effect::NoteCut(0) => self.volume = 0,
            Effect::SetVibratoWaveform(w) => self.vibrato_waveform = *w,
            Effect::SetTremoloWaveform(w) => self.tremolo_waveform = *w,
            _ => {}
        }
    }

    /// Clear temporary per-tick modulation before applying effects.
    pub fn clear_modulation(&mut self) {
        self.period_offset = 0;
        self.volume_offset = 0;
    }

    /// Advance the period modulator and apply based on mode.
    fn advance_period_mod(&mut self, spt: u32) {
        if let Some(m) = &mut self.period_mod {
            let prev = self.period;
            m.state.advance(&m.envelope, spt);
            match m.mode {
                ModMode::Set => {
                    let mut p = clamp_period(m.state.value() as u16);
                    // Prevent overshoot for tone portamento
                    if self.target_period > 0 {
                        p = clamp_toward(p, prev, self.target_period);
                    }
                    self.period = p;
                }
                ModMode::Add => self.period_offset = m.state.value() as i16,
                _ => {}
            }
        }
    }

    /// Advance the volume modulator and apply based on mode.
    fn advance_volume_mod(&mut self, spt: u32) {
        if let Some(m) = &mut self.volume_mod {
            m.state.advance(&m.envelope, spt);
            match m.mode {
                ModMode::Set => self.volume = m.state.value().clamp(0.0, 64.0) as u8,
                ModMode::Add => self.volume_offset = m.state.value() as i8,
                _ => {}
            }
        }
    }

    /// Advance the trigger modulator and reset position on loop.
    fn advance_trigger_mod(&mut self, spt: u32) {
        if let Some(m) = &mut self.trigger_mod {
            m.state.advance(&m.envelope, spt);
            if m.state.looped() {
                self.position = 0;
            }
        }
    }

    /// Advance all active modulators (called every tick).
    pub fn advance_modulators(&mut self, spt: u32) {
        self.advance_period_mod(spt);
        self.advance_volume_mod(spt);
        self.advance_trigger_mod(spt);
    }

    /// Set up envelope-based modulators for the current effect.
    /// Called when a new per-tick effect is dispatched.
    pub fn setup_modulator(&mut self, effect: &Effect, spt: u32) {
        match effect {
            Effect::VolumeSlide(delta) => {
                let env = volume_slide_envelope(self.volume as f32, *delta as f32, spt);
                self.volume_mod = Some(ActiveMod::new(env, ModMode::Set));
                self.period_mod = None;
                self.trigger_mod = None;
            }
            Effect::PortaUp(v) => {
                self.target_period = 0; // clear tone porta target
                let env = porta_envelope(
                    self.period as f32, -(*v as f32),
                    PERIOD_MIN as f32, PERIOD_MAX as f32, spt,
                );
                self.period_mod = Some(ActiveMod::new(env, ModMode::Set));
                self.volume_mod = None;
                self.trigger_mod = None;
            }
            Effect::PortaDown(v) => {
                self.target_period = 0; // clear tone porta target
                let env = porta_envelope(
                    self.period as f32, *v as f32,
                    PERIOD_MIN as f32, PERIOD_MAX as f32, spt,
                );
                self.period_mod = Some(ActiveMod::new(env, ModMode::Set));
                self.volume_mod = None;
                self.trigger_mod = None;
            }
            Effect::TonePorta(_speed) => {
                let env = tone_porta_envelope(
                    self.period as f32, self.target_period as f32,
                    self.porta_speed as f32, spt,
                );
                self.period_mod = Some(ActiveMod::new(env, ModMode::Set));
                self.volume_mod = None;
                self.trigger_mod = None;
            }
            Effect::TonePortaVolSlide(delta) => {
                let period_env = tone_porta_envelope(
                    self.period as f32, self.target_period as f32,
                    self.porta_speed as f32, spt,
                );
                self.period_mod = Some(ActiveMod::new(period_env, ModMode::Set));
                let vol_env = volume_slide_envelope(self.volume as f32, *delta as f32, spt);
                self.volume_mod = Some(ActiveMod::new(vol_env, ModMode::Set));
                self.trigger_mod = None;
            }
            Effect::Vibrato { speed, depth } => {
                let s = if *speed > 0 { *speed } else { self.vibrato_speed };
                let d = if *depth > 0 { *depth } else { self.vibrato_depth };
                if *speed > 0 { self.vibrato_speed = s; }
                if *depth > 0 { self.vibrato_depth = d; }
                self.period_mod = build_add_mode_sine_mod(s, d, spt);
                self.volume_mod = None;
                self.trigger_mod = None;
            }
            Effect::VibratoVolSlide(delta) => {
                // Keep existing period_mod (vibrato continues from previous row)
                // If no vibrato mod exists, create one from stored params
                if self.period_mod.is_none() && self.vibrato_speed > 0 {
                    self.period_mod =
                        build_add_mode_sine_mod(self.vibrato_speed, self.vibrato_depth, spt);
                }
                let vol_env = volume_slide_envelope(self.volume as f32, *delta as f32, spt);
                self.volume_mod = Some(ActiveMod::new(vol_env, ModMode::Set));
                self.trigger_mod = None;
            }
            Effect::Tremolo { speed, depth } => {
                let s = if *speed > 0 { *speed } else { self.tremolo_speed };
                let d = if *depth > 0 { *depth } else { self.tremolo_depth };
                if *speed > 0 { self.tremolo_speed = s; }
                if *depth > 0 { self.tremolo_depth = d; }
                self.volume_mod = build_add_mode_sine_mod(s, d, spt);
                self.period_mod = None;
                self.trigger_mod = None;
            }
            Effect::Arpeggio { x, y } => {
                self.period_mod = build_arpeggio_mod(self.note, self.period, *x, *y, spt);
                self.volume_mod = None;
                self.trigger_mod = None;
            }
            Effect::NoteCut(tick) if *tick > 0 => {
                let env = note_cut_envelope(self.volume as f32, *tick, spt);
                self.volume_mod = Some(ActiveMod::new(env, ModMode::Set));
                self.period_mod = None;
                self.trigger_mod = None;
            }
            Effect::RetriggerNote(interval) if *interval > 0 => {
                let env = retrigger_envelope(*interval, spt);
                self.trigger_mod = Some(ActiveMod::new(env, ModMode::Trigger));
                self.period_mod = None;
                self.volume_mod = None;
            }
            _ => {
                // Non-modulator effects: clear all mods
                self.period_mod = None;
                self.volume_mod = None;
                self.trigger_mod = None;
            }
        }
    }

    pub fn render(&mut self, sample: &Sample) -> Frame {
        // Read sample value with linear interpolation
        let sample_value = sample.data.get_mono_interpolated(self.position);

        // Apply volume (with tremolo offset) and panning
        // pan: -64 (full left) to +64 (full right)
        // Convert to 0..128 range for linear crossfade
        let vol = (self.volume as i32 + self.volume_offset as i32).clamp(0, 64);
        let pan_right = self.panning as i32 + 64; // 0..128
        let left_vol = ((128 - pan_right) * vol) >> 7;
        let right_vol = (pan_right * vol) >> 7;

        let left = (sample_value as i32 * left_vol) >> 6;
        let right = (sample_value as i32 * right_vol) >> 6;

        // Advance position
        self.position += self.increment;

        // Handle looping
        let pos_samples = self.position >> 16;
        if sample.has_loop() && pos_samples >= sample.loop_end {
            let loop_len = sample.loop_end - sample.loop_start;
            self.position -= loop_len << 16;
        } else if pos_samples >= sample.len() as u32 {
            self.playing = false;
        }

        Frame {
            left: left.clamp(-32768, 32767) as i16,
            right: right.clamp(-32768, 32767) as i16,
        }
    }

}

/// Clamp `value` so it doesn't overshoot `target` relative to `prev`.
/// If `prev` was above `target` and `value` went below it, return `target`.
/// If `prev` was below `target` and `value` went above it, return `target`.
fn clamp_toward(value: u16, prev: u16, target: u16) -> u16 {
    if prev > target && value < target {
        target
    } else if prev < target && value > target {
        target
    } else {
        value
    }
}

fn build_add_mode_sine_mod(speed: u8, depth: u8, spt: u32) -> Option<ActiveMod> {
    if speed == 0 && depth == 0 {
        return None;
    }
    let env = add_mode_sine_envelope(speed, depth, spt);
    Some(ActiveMod::new(env, ModMode::Add))
}

fn build_arpeggio_mod(note: u8, period: u16, x: u8, y: u8, spt: u32) -> Option<ActiveMod> {
    let offset_x = if x == 0 {
        0.0
    } else {
        let target = note_to_period(note.saturating_add(x));
        if target > 0 { target as f32 - period as f32 } else { 0.0 }
    };
    let offset_y = if y == 0 {
        0.0
    } else {
        let target = note_to_period(note.saturating_add(y));
        if target > 0 { target as f32 - period as f32 } else { 0.0 }
    };
    let env = arpeggio_envelope([0.0, offset_x, offset_y], spt);
    Some(ActiveMod::new(env, ModMode::Add))
}
