//! Channel state for tracker playback.

use mb_ir::Effect;

use crate::frequency::{clamp_period, note_to_period, period_to_increment, PERIOD_MAX, PERIOD_MIN};

/// ProTracker sine table: 32-entry half-wave (0→255→0).
/// Phase 0-63, index = phase & 31, sign = phase & 32.
const SINE_TABLE: [u8; 32] = [
    0, 24, 49, 74, 97, 120, 141, 161, 180, 197, 212, 224, 235, 244, 250, 255,
    255, 250, 244, 235, 224, 212, 197, 180, 161, 141, 120, 97, 74, 49, 24, 0,
];

/// Compute signed waveform value for vibrato/tremolo.
/// Returns -255..255 based on waveform type and phase (0-63).
fn waveform_value(waveform: u8, phase: u8) -> i16 {
    let index = (phase & 31) as usize;
    let magnitude = match waveform & 3 {
        0 => SINE_TABLE[index] as i16,
        1 => (index as i16) << 3, // ramp: 0, 8, 16, ..., 248
        _ => 255i16,              // square
    };
    if phase & 32 != 0 { -magnitude } else { magnitude }
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
    /// Left channel volume (0-64)
    pub volume_left: u8,
    /// Right channel volume (0-64)
    pub volume_right: u8,
    /// Is the channel currently playing?
    pub playing: bool,

    // Effect state
    /// Currently active per-tick effect
    pub active_effect: Effect,
    /// Vibrato phase (0-255)
    pub vibrato_phase: u8,
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

    // Vibrato state
    /// Vibrato speed (phase advance per tick)
    pub vibrato_speed: u8,
    /// Vibrato depth (period modulation amplitude)
    pub vibrato_depth: u8,
    /// Vibrato waveform (0=sine, 1=ramp, 2=square; bit 2=no retrig)
    pub vibrato_waveform: u8,

    // Tremolo state
    /// Tremolo speed (phase advance per tick)
    pub tremolo_speed: u8,
    /// Tremolo depth (volume modulation amplitude)
    pub tremolo_depth: u8,
    /// Tremolo phase (0-63)
    pub tremolo_phase: u8,
    /// Tremolo waveform (0=sine, 1=ramp, 2=square; bit 2=no retrig)
    pub tremolo_waveform: u8,

    // Arpeggio state
    /// Arpeggio tick counter (cycles 0→1→2→0)
    pub arpeggio_tick: u8,

    // Temporary per-tick modulation (not saved to base values)
    /// Period offset from vibrato/arpeggio
    pub period_offset: i16,
    /// Volume offset from tremolo
    pub volume_offset: i8,

    /// Tick counter for the current active effect (increments each process_tick)
    pub effect_tick: u8,
}

impl ChannelState {
    /// Create a new channel state.
    pub fn new() -> Self {
        Self {
            volume: 64,
            volume_left: 64,
            volume_right: 64,
            loop_forward: true,
            ..Default::default()
        }
    }

    /// Reset the channel to initial state.
    pub fn reset(&mut self) {
        *self = Self::new();
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
        self.active_effect = Effect::None;
        self.effect_tick = 0;
        self.period_offset = 0;
        self.volume_offset = 0;
        self.arpeggio_tick = 0;
        // Retrigger waveform phase unless bit 2 is set
        if self.vibrato_waveform & 4 == 0 {
            self.vibrato_phase = 0;
        }
        if self.tremolo_waveform & 4 == 0 {
            self.tremolo_phase = 0;
        }
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
            Effect::SampleOffset(o) => self.position = (*o as u32) << 24, // 256-byte units → 16.16
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

    /// Slide period toward target_period by porta_speed.
    fn apply_tone_porta(&mut self) {
        if self.target_period == 0 || self.period == 0 {
            return;
        }
        if self.period > self.target_period {
            self.period = self.period.saturating_sub(self.porta_speed as u16);
            if self.period < self.target_period {
                self.period = self.target_period;
            }
        } else if self.period < self.target_period {
            self.period = self.period.saturating_add(self.porta_speed as u16);
            if self.period > self.target_period {
                self.period = self.target_period;
            }
        }
    }

    /// Apply vibrato: oscillate period around base value.
    fn apply_vibrato(&mut self) {
        let delta = waveform_value(self.vibrato_waveform, self.vibrato_phase);
        self.period_offset = (delta as i32 * self.vibrato_depth as i32 / 128) as i16;
        self.vibrato_phase = (self.vibrato_phase.wrapping_add(self.vibrato_speed)) & 63;
    }

    /// Apply tremolo: oscillate volume around base value.
    fn apply_tremolo(&mut self) {
        let delta = waveform_value(self.tremolo_waveform, self.tremolo_phase);
        self.volume_offset = (delta as i32 * self.tremolo_depth as i32 / 64) as i8;
        self.tremolo_phase = (self.tremolo_phase.wrapping_add(self.tremolo_speed)) & 63;
    }

    /// Apply arpeggio: cycle period between base note, +x, +y semitones.
    fn apply_arpeggio(&mut self, x: u8, y: u8) {
        let semitone_offset = match self.arpeggio_tick % 3 {
            1 => x,
            2 => y,
            _ => 0,
        };
        self.period_offset = if semitone_offset == 0 {
            0
        } else {
            let target = note_to_period(self.note.saturating_add(semitone_offset));
            if target > 0 { target as i16 - self.period as i16 } else { 0 }
        };
        self.arpeggio_tick = (self.arpeggio_tick + 1) % 3;
    }

    /// Clear temporary per-tick modulation before applying effects.
    pub fn clear_modulation(&mut self) {
        self.period_offset = 0;
        self.volume_offset = 0;
    }

    /// Apply a per-tick effect (called every tick after the first).
    pub fn apply_tick_effect(&mut self) {
        self.effect_tick = self.effect_tick.wrapping_add(1);
        match self.active_effect {
            Effect::VolumeSlide(delta) => {
                self.volume = (self.volume as i16 + delta as i16).clamp(0, 64) as u8;
            }
            Effect::PortaUp(v) => {
                self.period = clamp_period(self.period.saturating_sub(v as u16));
            }
            Effect::PortaDown(v) => {
                self.period = clamp_period(self.period.saturating_add(v as u16));
            }
            Effect::TonePorta(_) => {
                self.apply_tone_porta();
            }
            Effect::TonePortaVolSlide(delta) => {
                self.apply_tone_porta();
                self.volume = (self.volume as i16 + delta as i16).clamp(0, 64) as u8;
            }
            Effect::Vibrato { speed, depth } => {
                if speed > 0 { self.vibrato_speed = speed; }
                if depth > 0 { self.vibrato_depth = depth; }
                self.apply_vibrato();
            }
            Effect::VibratoVolSlide(delta) => {
                self.apply_vibrato();
                self.volume = (self.volume as i16 + delta as i16).clamp(0, 64) as u8;
            }
            Effect::Tremolo { speed, depth } => {
                if speed > 0 { self.tremolo_speed = speed; }
                if depth > 0 { self.tremolo_depth = depth; }
                self.apply_tremolo();
            }
            Effect::Arpeggio { x, y } => {
                self.apply_arpeggio(x, y);
            }
            Effect::NoteCut(tick) => {
                if self.effect_tick >= tick {
                    self.volume = 0;
                }
            }
            Effect::RetriggerNote(interval) => {
                if interval > 0 && self.effect_tick % interval == 0 {
                    self.position = 0;
                }
            }
            _ => {}
        }
    }
}
