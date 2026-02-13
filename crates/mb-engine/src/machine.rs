//! Machine trait for audio generators and effects.

/// Work mode indicating data flow direction for `Machine::work()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkMode {
    /// Called but no input/output expected.
    NoIO,
    /// Input available, not writing (generator silent).
    Read,
    /// No input, writing output (generator active).
    Write,
    /// Normal: reading input, writing output.
    ReadWrite,
}

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
pub trait Machine: Send {
    fn info(&self) -> &MachineInfo;
    fn init(&mut self, sample_rate: u32);
    fn tick(&mut self);
    fn work(&mut self, buffer: &mut [f32], mode: WorkMode) -> bool;
    fn stop(&mut self);
    fn set_param(&mut self, param: u16, value: i32);
}
