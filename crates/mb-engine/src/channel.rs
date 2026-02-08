//! Channel state for tracker playback.

use mb_ir::Effect;

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
    /// Target increment for tone portamento (16.16 fixed-point)
    pub porta_target: u32,
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
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.playing = false;
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
            Effect::NoteCut(0) => self.volume = 0,
            _ => {}
        }
    }

    /// Apply a per-tick effect (called every tick after the first).
    pub fn apply_tick_effect(&mut self) {
        match self.active_effect {
            Effect::VolumeSlide(delta) => {
                self.volume = (self.volume as i16 + delta as i16).clamp(0, 64) as u8;
            }
            Effect::TonePortaVolSlide(delta) | Effect::VibratoVolSlide(delta) => {
                self.volume = (self.volume as i16 + delta as i16).clamp(0, 64) as u8;
            }
            Effect::NoteCut(tick) => {
                // NoteCut is tracked as active_effect; decrement handled externally
                // For simplicity, we just store it and let process_tick handle timing
                let _ = tick;
            }
            _ => {}
        }
    }
}
