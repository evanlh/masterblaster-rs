//! Channel state for tracker playback.

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
    /// Target note for tone portamento
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
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.playing = false;
    }
}
