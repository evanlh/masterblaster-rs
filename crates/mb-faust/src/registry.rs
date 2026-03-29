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
        _ => return None,
    };
    FaustMachine::from_source(dsp_name, source)
        .ok()
        .map(|m| Box::new(m) as Box<dyn Machine>)
}
