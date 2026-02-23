//! Passthrough machine — placeholder for unimplemented Buzz machines.
//!
//! Passes input audio to output unchanged. Used so the graph shape matches
//! the original Buzz song exactly, with every node having a machine instance.

use mb_ir::{AudioBuffer, AudioStream, ChannelConfig};
use crate::machine::{Machine, MachineInfo, MachineType};

static INFO: MachineInfo = MachineInfo {
    name: "Passthrough",
    short_name: "Pass",
    author: "masterblaster",
    machine_type: MachineType::Effect,
    params: &[],
};

pub struct PassthroughMachine;

impl AudioStream for PassthroughMachine {
    fn channel_config(&self) -> ChannelConfig {
        ChannelConfig { inputs: 2, outputs: 2 }
    }

    fn render(&mut self, _output: &mut AudioBuffer) {
        // No-op: data already in buffer from gather_inputs
    }
}

impl Machine for PassthroughMachine {
    fn info(&self) -> &MachineInfo { &INFO }
    fn init(&mut self, _sample_rate: u32) {}
    fn tick(&mut self) {}
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
        let mut buf = AudioBuffer::new(2, 2);
        buf.channel_mut(0)[0] = 0.5;
        buf.channel_mut(0)[1] = -0.3;
        buf.channel_mut(1)[0] = 0.8;
        buf.channel_mut(1)[1] = -0.1;

        let original_l: Vec<f32> = buf.channel(0).to_vec();
        let original_r: Vec<f32> = buf.channel(1).to_vec();

        m.render(&mut buf);

        assert_eq!(buf.channel(0), original_l.as_slice());
        assert_eq!(buf.channel(1), original_r.as_slice());
    }
}
