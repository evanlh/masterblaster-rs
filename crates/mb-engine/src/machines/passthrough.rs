//! Passthrough machine — placeholder for unimplemented Buzz machines.
//!
//! Passes input audio to output unchanged. Used so the graph shape matches
//! the original Buzz song exactly, with every node having a machine instance.

use crate::machine::{Machine, MachineInfo, MachineType, WorkMode};

static INFO: MachineInfo = MachineInfo {
    name: "Passthrough",
    short_name: "Pass",
    author: "masterblaster",
    machine_type: MachineType::Effect,
    params: &[],
};

pub struct PassthroughMachine;

impl Machine for PassthroughMachine {
    fn info(&self) -> &MachineInfo { &INFO }
    fn init(&mut self, _sample_rate: u32) {}
    fn tick(&mut self) {}
    fn work(&mut self, _buffer: &mut [f32], _mode: WorkMode) -> bool { true }
    fn stop(&mut self) {}
    fn set_param(&mut self, _param: u16, _value: i32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_leaves_buffer_unchanged() {
        let mut m = PassthroughMachine;
        m.init(44100);
        let mut buf = [0.5f32, -0.3, 0.8, -0.1];
        let original = buf;
        m.work(&mut buf, WorkMode::ReadWrite);
        assert_eq!(buf, original);
    }
}
