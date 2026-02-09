//! Cell formatting — unchanged from egui version.

pub fn format_cell(cell: &mb_ir::Cell) -> String {
    format!(
        "{} {} {}",
        format_note(cell.note),
        format_instrument(cell.instrument),
        format_effect(&cell.effect),
    )
}

pub fn format_note(note: mb_ir::Note) -> &'static str {
    match note {
        mb_ir::Note::None => "---",
        mb_ir::Note::Off => "===",
        mb_ir::Note::Fade => "^^^",
        mb_ir::Note::On(n) => note_name(n),
    }
}

fn note_name(n: u8) -> &'static str {
    const NAMES: [&str; 120] = [
        "C-0","C#0","D-0","D#0","E-0","F-0","F#0","G-0","G#0","A-0","A#0","B-0",
        "C-1","C#1","D-1","D#1","E-1","F-1","F#1","G-1","G#1","A-1","A#1","B-1",
        "C-2","C#2","D-2","D#2","E-2","F-2","F#2","G-2","G#2","A-2","A#2","B-2",
        "C-3","C#3","D-3","D#3","E-3","F-3","F#3","G-3","G#3","A-3","A#3","B-3",
        "C-4","C#4","D-4","D#4","E-4","F-4","F#4","G-4","G#4","A-4","A#4","B-4",
        "C-5","C#5","D-5","D#5","E-5","F-5","F#5","G-5","G#5","A-5","A#5","B-5",
        "C-6","C#6","D-6","D#6","E-6","F-6","F#6","G-6","G#6","A-6","A#6","B-6",
        "C-7","C#7","D-7","D#7","E-7","F-7","F#7","G-7","G#7","A-7","A#7","B-7",
        "C-8","C#8","D-8","D#8","E-8","F-8","F#8","G-8","G#8","A-8","A#8","B-8",
        "C-9","C#9","D-9","D#9","E-9","F-9","F#9","G-9","G#9","A-9","A#9","B-9",
    ];
    NAMES.get(n as usize).unwrap_or(&"???")
}

pub fn format_instrument(inst: u8) -> String {
    if inst > 0 {
        format!("{:02X}", inst)
    } else {
        "..".to_string()
    }
}

pub fn format_effect(effect: &mb_ir::Effect) -> String {
    use mb_ir::Effect::*;
    match effect {
        None => "...".to_string(),
        Arpeggio { x, y } => format!("0{:X}{:X}", x, y),
        PortaUp(v) => format!("1{:02X}", v),
        PortaDown(v) => format!("2{:02X}", v),
        TonePorta(v) => format!("3{:02X}", v),
        Vibrato { speed, depth } => format!("4{:X}{:X}", speed, depth),
        TonePortaVolSlide(v) => format!("5{}", vol_slide_param(*v)),
        VibratoVolSlide(v) => format!("6{}", vol_slide_param(*v)),
        Tremolo { speed, depth } => format!("7{:X}{:X}", speed, depth),
        SetPan(v) => format!("8{:02X}", v),
        SampleOffset(v) => format!("9{:02X}", v),
        VolumeSlide(v) => format!("A{}", vol_slide_param(*v)),
        PositionJump(v) => format!("B{:02X}", v),
        SetVolume(v) => format!("C{:02X}", v),
        PatternBreak(v) => format!("D{:02X}", v),
        FinePortaUp(v) => format!("E1{:X}", v),
        FinePortaDown(v) => format!("E2{:X}", v),
        SetVibratoWaveform(v) => format!("E4{:X}", v),
        SetFinetune(v) => format!("E5{:X}", *v as u8 & 0xF),
        PatternLoop(v) => format!("E6{:X}", v),
        SetTremoloWaveform(v) => format!("E7{:X}", v),
        SetPanPosition(v) => format!("E8{:X}", v),
        RetriggerNote(v) => format!("E9{:X}", v),
        FineVolumeSlideUp(v) => format!("EA{:X}", v),
        FineVolumeSlideDown(v) => format!("EB{:X}", v),
        NoteCut(v) => format!("EC{:X}", v),
        NoteDelay(v) => format!("ED{:X}", v),
        PatternDelay(v) => format!("EE{:X}", v),
        SetSpeed(v) => format!("F{:02X}", v),
        SetTempo(v) => format!("F{:02X}", v),
        other => format!("{:.3}", other.name()),
    }
}

fn vol_slide_param(v: i8) -> String {
    if v >= 0 {
        format!("{:X}0", v)
    } else {
        format!("0{:X}", -v)
    }
}
