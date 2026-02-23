//! Machine trait for audio generators and effects.

use mb_ir::AudioStream;

/// Whether a machine generates or processes audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MachineType {
    Generator,
    Effect,
}

/// Metadata describing a machine's parameters.
pub struct ParamInfo {
    pub id: u16,
    pub name: &'static str,
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub no_value: i32,
}

/// Static metadata about a machine.
pub struct MachineInfo {
    pub name: &'static str,
    pub short_name: &'static str,
    pub author: &'static str,
    pub machine_type: MachineType,
    pub params: &'static [ParamInfo],
}

/// Core trait for audio generators and effects.
///
/// Extends `AudioStream` for buffer-based rendering.
pub trait Machine: AudioStream + Send {
    fn info(&self) -> &MachineInfo;
    fn init(&mut self, sample_rate: u32);
    fn tick(&mut self);
    fn stop(&mut self);
    fn set_param(&mut self, param: u16, value: i32);
}
