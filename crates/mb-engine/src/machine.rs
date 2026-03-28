//! Machine trait for audio generators and effects.

use alloc::string::String;
use alloc::vec::Vec;

use mb_ir::{AudioStream, EventPayload};

/// Whether a machine generates or processes audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MachineType {
    Generator,
    Effect,
}

/// Metadata describing a machine's parameters.
pub struct ParamInfo {
    pub id: u16,
    pub name: String,
    pub min: i32,
    pub max: i32,
    pub default: i32,
    pub no_value: i32,
}

impl ParamInfo {
    pub fn new(id: u16, name: &str, min: i32, max: i32, default: i32) -> Self {
        Self { id, name: String::from(name), min, max, default, no_value: 0 }
    }
}

/// Metadata about a machine.
pub struct MachineInfo {
    pub name: String,
    pub short_name: String,
    pub author: String,
    pub machine_type: MachineType,
    pub params: Vec<ParamInfo>,
}

impl MachineInfo {
    pub fn new(name: &str, short_name: &str, author: &str, machine_type: MachineType, params: Vec<ParamInfo>) -> Self {
        Self {
            name: String::from(name),
            short_name: String::from(short_name),
            author: String::from(author),
            machine_type,
            params,
        }
    }
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

    /// Dispatch a channel event to a sub-channel within this machine.
    fn apply_event(&mut self, _channel: u8, _payload: &EventPayload) {}

    /// Notify the machine of a speed change (ticks per row).
    fn set_speed(&mut self, _speed: u8) {}
}
