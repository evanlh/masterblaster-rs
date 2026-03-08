//! Cell formatting using a reusable scratch buffer to avoid per-cell heap allocations.

use core::fmt::Write;

/// Write a formatted cell into the given buffer (clears it first).
pub fn format_cell_into(cell: &mb_ir::Cell, buf: &mut String) {
    buf.clear();
    write_note(buf, cell.note);
    buf.push(' ');
    write_instrument(buf, cell.instrument);
    buf.push(' ');
    write_effect(buf, &cell.effect);
}

pub fn format_note(note: mb_ir::Note) -> &'static str {
    match note {
        mb_ir::Note::None => "---",
        mb_ir::Note::Off => "===",
        mb_ir::Note::Fade => "^^^",
        mb_ir::Note::On(n) => note_name(n),
    }
}

fn write_note(buf: &mut String, note: mb_ir::Note) {
    buf.push_str(format_note(note));
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

fn write_instrument(buf: &mut String, inst: u8) {
    if inst > 0 {
        let _ = write!(buf, "{:02X}", inst);
    } else {
        buf.push_str("..");
    }
}

fn write_effect(buf: &mut String, effect: &mb_ir::Effect) {
    use mb_ir::Effect::*;
    match effect {
        None => buf.push_str("..."),
        Arpeggio { x, y } => { let _ = write!(buf, "0{:X}{:X}", x, y); }
        PortaUp(v) => { let _ = write!(buf, "1{:02X}", v); }
        PortaDown(v) => { let _ = write!(buf, "2{:02X}", v); }
        TonePorta(v) => { let _ = write!(buf, "3{:02X}", v); }
        Vibrato { speed, depth } => { let _ = write!(buf, "4{:X}{:X}", speed, depth); }
        TonePortaVolSlide(v) => { write_vol_slide_effect(buf, '5', *v); }
        VibratoVolSlide(v) => { write_vol_slide_effect(buf, '6', *v); }
        Tremolo { speed, depth } => { let _ = write!(buf, "7{:X}{:X}", speed, depth); }
        SetPan(v) => { let _ = write!(buf, "8{:02X}", v); }
        SampleOffset(v) | FractionalSampleOffset(v) => { let _ = write!(buf, "9{:02X}", v); }
        VolumeSlide(v) => { write_vol_slide_effect(buf, 'A', *v); }
        PositionJump(v) => { let _ = write!(buf, "B{:02X}", v); }
        SetVolume(v) => { let _ = write!(buf, "C{:02X}", v); }
        PatternBreak(v) => { let _ = write!(buf, "D{:02X}", v); }
        FinePortaUp(v) => { let _ = write!(buf, "E1{:X}", v); }
        FinePortaDown(v) => { let _ = write!(buf, "E2{:X}", v); }
        SetVibratoWaveform(v) => { let _ = write!(buf, "E4{:X}", v); }
        SetFinetune(v) => { let _ = write!(buf, "E5{:X}", *v as u8 & 0xF); }
        PatternLoop(v) => { let _ = write!(buf, "E6{:X}", v); }
        SetTremoloWaveform(v) => { let _ = write!(buf, "E7{:X}", v); }
        SetPanPosition(v) => { let _ = write!(buf, "E8{:X}", v); }
        RetriggerNote(v) => { let _ = write!(buf, "E9{:X}", v); }
        FineVolumeSlideUp(v) => { let _ = write!(buf, "EA{:X}", v); }
        FineVolumeSlideDown(v) => { let _ = write!(buf, "EB{:X}", v); }
        NoteCut(v) => { let _ = write!(buf, "EC{:X}", v); }
        NoteDelay(v) => { let _ = write!(buf, "ED{:X}", v); }
        PatternDelay(v) => { let _ = write!(buf, "EE{:X}", v); }
        SetSpeed(v) => { let _ = write!(buf, "F{:02X}", v); }
        SetTempo(v) => { let _ = write!(buf, "F{:02X}", v); }
        other => {
            let name = other.name();
            // Take up to 3 chars
            let truncated: String = name.chars().take(3).collect();
            buf.push_str(&truncated);
        }
    }
}

fn write_vol_slide_effect(buf: &mut String, prefix: char, v: i8) {
    buf.push(prefix);
    if v >= 0 {
        let _ = write!(buf, "{:X}0", v);
    } else {
        let _ = write!(buf, "0{:X}", -v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format_cell(cell: &mb_ir::Cell) -> String {
        let mut buf = String::with_capacity(16);
        format_cell_into(cell, &mut buf);
        buf
    }

    #[test]
    fn empty_cell() {
        let cell = mb_ir::Cell::empty();
        assert_eq!(format_cell(&cell), "--- .. ...");
    }

    fn cell(note: mb_ir::Note, instrument: u8, effect: mb_ir::Effect) -> mb_ir::Cell {
        mb_ir::Cell { note, instrument, effect, ..mb_ir::Cell::empty() }
    }

    #[test]
    fn note_on_with_instrument_and_effect() {
        assert_eq!(format_cell(&cell(mb_ir::Note::On(48), 1, mb_ir::Effect::SetVolume(64))), "C-4 01 C40");
    }

    #[test]
    fn note_off() {
        assert_eq!(format_cell(&cell(mb_ir::Note::Off, 0, mb_ir::Effect::None)), "=== .. ...");
    }

    #[test]
    fn volume_slide_up() {
        assert_eq!(format_cell(&cell(mb_ir::Note::None, 0, mb_ir::Effect::VolumeSlide(3))), "--- .. A30");
    }

    #[test]
    fn volume_slide_down() {
        assert_eq!(format_cell(&cell(mb_ir::Note::None, 0, mb_ir::Effect::VolumeSlide(-5))), "--- .. A05");
    }

    #[test]
    fn scratch_buffer_reuse() {
        let mut buf = String::with_capacity(16);
        let cell1 = cell(mb_ir::Note::On(48), 1, mb_ir::Effect::SetVolume(64));

        format_cell_into(&cell1, &mut buf);
        assert_eq!(buf, "C-4 01 C40");

        format_cell_into(&mb_ir::Cell::empty(), &mut buf);
        assert_eq!(buf, "--- .. ...");
    }

    #[test]
    fn arpeggio_format() {
        assert_eq!(format_cell(&cell(mb_ir::Note::None, 0, mb_ir::Effect::Arpeggio { x: 3, y: 7 })), "--- .. 037");
    }

    #[test]
    fn porta_up_format() {
        assert_eq!(format_cell(&cell(mb_ir::Note::None, 0, mb_ir::Effect::PortaUp(15))), "--- .. 10F");
    }

    #[test]
    fn sample_offset_format() {
        assert_eq!(format_cell(&cell(mb_ir::Note::On(60), 2, mb_ir::Effect::SampleOffset(128))), "C-5 02 980");
    }
}
