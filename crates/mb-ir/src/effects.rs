//! Effect command types for tracker patterns.

/// Volume column command (XM/IT style).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VolumeCommand {
    #[default]
    None,
    /// Set volume (0-64)
    Volume(u8),
    VolumeSlideDown(u8),
    VolumeSlideUp(u8),
    FineVolSlideDown(u8),
    FineVolSlideUp(u8),
    /// Set panning (0-64, 32 = center)
    Panning(u8),
    PortaDown(u8),
    PortaUp(u8),
    TonePorta(u8),
    Vibrato(u8),
}

impl VolumeCommand {
    /// Returns the variant name as a static string (ignoring parameters).
    pub fn name(&self) -> &'static str {
        match self {
            VolumeCommand::None => "None",
            VolumeCommand::Volume(_) => "Volume",
            VolumeCommand::VolumeSlideDown(_) => "VolumeSlideDown",
            VolumeCommand::VolumeSlideUp(_) => "VolumeSlideUp",
            VolumeCommand::FineVolSlideDown(_) => "FineVolSlideDown",
            VolumeCommand::FineVolSlideUp(_) => "FineVolSlideUp",
            VolumeCommand::Panning(_) => "Panning",
            VolumeCommand::PortaDown(_) => "PortaDown",
            VolumeCommand::PortaUp(_) => "PortaUp",
            VolumeCommand::TonePorta(_) => "TonePorta",
            VolumeCommand::Vibrato(_) => "Vibrato",
        }
    }
}

/// Effect column command.
///
/// This enum covers effects from MOD, S3M, XM, and IT formats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effect {
    #[default]
    None,

    // === Arpeggio & Portamento ===
    /// Arpeggio: cycle between note, note+x, note+y each tick
    Arpeggio { x: u8, y: u8 },
    /// Slide pitch up by amount per tick
    PortaUp(u8),
    /// Slide pitch down by amount per tick
    PortaDown(u8),
    /// Slide toward target note
    TonePorta(u8),
    /// Vibrato with speed and depth
    Vibrato { speed: u8, depth: u8 },
    /// Tone portamento + volume slide
    TonePortaVolSlide(i8),
    /// Vibrato + volume slide
    VibratoVolSlide(i8),

    // === Tremolo & Volume ===
    /// Tremolo (volume oscillation)
    Tremolo { speed: u8, depth: u8 },
    /// Set channel panning (0-255)
    SetPan(u8),
    /// Set sample offset (in 256-byte units)
    SampleOffset(u8),
    /// Volume slide up/down per tick
    VolumeSlide(i8),
    /// Jump to order position
    PositionJump(u8),
    /// Set channel volume (0-64)
    SetVolume(u8),
    /// Break to row in next pattern
    PatternBreak(u8),

    // === Extended effects (Exx/Fxx style) ===
    /// Fine porta up (once per row)
    FinePortaUp(u8),
    /// Fine porta down (once per row)
    FinePortaDown(u8),
    /// Set vibrato waveform (0=sine, 1=ramp, 2=square)
    SetVibratoWaveform(u8),
    /// Set finetune (-8 to +7)
    SetFinetune(i8),
    /// Pattern loop (0=set start, n=loop n times)
    PatternLoop(u8),
    /// Set tremolo waveform
    SetTremoloWaveform(u8),
    /// Set panning position / surround
    SetPanPosition(u8),
    /// Retrigger note every n ticks
    RetriggerNote(u8),
    /// Fine volume slide up (once per row)
    FineVolumeSlideUp(u8),
    /// Fine volume slide down (once per row)
    FineVolumeSlideDown(u8),
    /// Cut note after n ticks
    NoteCut(u8),
    /// Delay note by n ticks
    NoteDelay(u8),
    /// Delay pattern by n rows
    PatternDelay(u8),

    // === Speed & Tempo ===
    /// Set ticks per row (speed)
    SetSpeed(u8),
    /// Set BPM tempo
    SetTempo(u8),

    // === IT-specific ===
    /// Set global volume (0-128)
    SetGlobalVolume(u8),
    /// Global volume slide
    GlobalVolumeSlide(i8),
    /// Set envelope position
    SetEnvelopePosition(u8),
    /// Panning slide
    PanningSlide(i8),
    /// Retrigger with volume change
    Retrigger { interval: u8, volume_change: i8 },
    /// Tremor (on/off volume)
    Tremor { on: u8, off: u8 },

    // === S3M-specific ===
    /// Set filter cutoff frequency
    SetFilterCutoff(u8),
    /// Set filter resonance
    SetFilterResonance(u8),

    // === Extra fine slides ===
    /// Extra fine porta up
    ExtraFinePortaUp(u8),
    /// Extra fine porta down
    ExtraFinePortaDown(u8),
}

impl Effect {
    /// Returns the variant name as a static string (ignoring parameters).
    pub fn name(&self) -> &'static str {
        match self {
            Effect::None => "None",
            Effect::Arpeggio { .. } => "Arpeggio",
            Effect::PortaUp(_) => "PortaUp",
            Effect::PortaDown(_) => "PortaDown",
            Effect::TonePorta(_) => "TonePorta",
            Effect::Vibrato { .. } => "Vibrato",
            Effect::TonePortaVolSlide(_) => "TonePortaVolSlide",
            Effect::VibratoVolSlide(_) => "VibratoVolSlide",
            Effect::Tremolo { .. } => "Tremolo",
            Effect::SetPan(_) => "SetPan",
            Effect::SampleOffset(_) => "SampleOffset",
            Effect::VolumeSlide(_) => "VolumeSlide",
            Effect::PositionJump(_) => "PositionJump",
            Effect::SetVolume(_) => "SetVolume",
            Effect::PatternBreak(_) => "PatternBreak",
            Effect::FinePortaUp(_) => "FinePortaUp",
            Effect::FinePortaDown(_) => "FinePortaDown",
            Effect::SetVibratoWaveform(_) => "SetVibratoWaveform",
            Effect::SetFinetune(_) => "SetFinetune",
            Effect::PatternLoop(_) => "PatternLoop",
            Effect::SetTremoloWaveform(_) => "SetTremoloWaveform",
            Effect::SetPanPosition(_) => "SetPanPosition",
            Effect::RetriggerNote(_) => "RetriggerNote",
            Effect::FineVolumeSlideUp(_) => "FineVolumeSlideUp",
            Effect::FineVolumeSlideDown(_) => "FineVolumeSlideDown",
            Effect::NoteCut(_) => "NoteCut",
            Effect::NoteDelay(_) => "NoteDelay",
            Effect::PatternDelay(_) => "PatternDelay",
            Effect::SetSpeed(_) => "SetSpeed",
            Effect::SetTempo(_) => "SetTempo",
            Effect::SetGlobalVolume(_) => "SetGlobalVolume",
            Effect::GlobalVolumeSlide(_) => "GlobalVolumeSlide",
            Effect::SetEnvelopePosition(_) => "SetEnvelopePosition",
            Effect::PanningSlide(_) => "PanningSlide",
            Effect::Retrigger { .. } => "Retrigger",
            Effect::Tremor { .. } => "Tremor",
            Effect::SetFilterCutoff(_) => "SetFilterCutoff",
            Effect::SetFilterResonance(_) => "SetFilterResonance",
            Effect::ExtraFinePortaUp(_) => "ExtraFinePortaUp",
            Effect::ExtraFinePortaDown(_) => "ExtraFinePortaDown",
        }
    }



    /// Returns true if this effect is processed only on tick 0.
    pub fn is_row_effect(&self) -> bool {
        matches!(self, Effect::NoteCut(0))
            || matches!(
                self,
                Effect::PositionJump(_)
                    | Effect::PatternBreak(_)
                    | Effect::SetSpeed(_)
                    | Effect::SetTempo(_)
                    | Effect::SetVolume(_)
                    | Effect::SetPan(_)
                    | Effect::SampleOffset(_)
                    | Effect::FinePortaUp(_)
                    | Effect::FinePortaDown(_)
                    | Effect::FineVolumeSlideUp(_)
                    | Effect::FineVolumeSlideDown(_)
                    | Effect::SetVibratoWaveform(_)
                    | Effect::SetTremoloWaveform(_)
                    | Effect::ExtraFinePortaUp(_)
                    | Effect::ExtraFinePortaDown(_)
                    | Effect::NoteDelay(_)
                    | Effect::PatternDelay(_)
            )
    }
}
