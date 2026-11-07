//! Registry mapping Buzz machine names to Faust DSP implementations.

use mb_engine::machine::Machine;

use crate::faust_machine::FaustMachine;

/// Look up a Faust DSP implementation for a Buzz machine name.
///
/// DSP sources are embedded at compile time via `include_str!`.
/// Returns `None` if no Faust implementation exists for the given name.
pub fn create_faust_machine(name: &str) -> Option<Box<dyn Machine>> {
    let (dsp_name, source) = match name {
        "Jeskola Filter 2" => ("filter2", include_str!("../../../faust/filter2.dsp")),
        "Jeskola Reverb 2" => ("reverb2", include_str!("../../../faust/reverb2.dsp")),
        "Jeskola Freeverb" => ("reverb", include_str!("../../../faust/reverb.dsp")),
        "FSM WahPro" => ("wahpro", include_str!("../../../faust/wahpro.dsp")),
        "Automaton Compressor" => (
            "automaton_compressor",
            include_str!("../../../faust/automaton_compressor.dsp"),
        ),
        "FSM WahPro2" => ("wahpro2", include_str!("../../../faust/wahpro2.dsp")),
        "11-Lofi v1.0" => ("lofi", include_str!("../../../faust/lofi.dsp")),
        "Arguelles Degrader" => ("degrader", include_str!("../../../faust/degrader.dsp")),
        "Argüelles KFlanger" => ("kflanger", include_str!("../../../faust/kflanger.dsp")),
        "asedev Psycho01" => ("psycho01", include_str!("../../../faust/psycho01.dsp")),
        "Automaton Compressor mkII" => (
            "automaton_compressor_mkii",
            include_str!("../../../faust/automaton_compressor_mkii.dsp"),
        ),
        _ => return None,
    };
    FaustMachine::from_source_cached(dsp_name, source)
        .ok()
        .map(|m| Box::new(m) as Box<dyn Machine>)
}
