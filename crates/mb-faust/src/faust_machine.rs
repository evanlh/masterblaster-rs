//! FaustMachine — Machine adapter for JIT-compiled Faust DSPs.

use mb_ir::{AudioBuffer, AudioStream, ChannelConfig};
use mb_engine::machine::{Machine, MachineInfo, MachineType};

use crate::compiler::CompiledDsp;
use crate::ffi;
use crate::ui_visitor;

/// A Machine wrapping a JIT-compiled Faust DSP.
pub struct FaustMachine {
    dsp: CompiledDsp,
    info: MachineInfo,
    sample_rate: u32,
    /// Scratch buffers for Faust inputs beyond AudioBuffer channels.
    extra_input_bufs: Vec<Vec<f32>>,
    output_bufs: Vec<Vec<f32>>,
}

// Safety: CompiledDsp is Send, and we only access from one thread.
unsafe impl Send for FaustMachine {}

impl FaustMachine {
    /// Create from a compiled Faust DSP.
    pub fn new(dsp: CompiledDsp) -> Self {
        let param_infos = ui_visitor::to_param_infos(dsp.params());
        let info = MachineInfo::new(
            "Faust DSP", "Faust", "Faust",
            MachineType::Effect,
            param_infos,
        );
        // Extra input buffers for Faust inputs beyond the 2-channel AudioBuffer
        let extra_count = dsp.num_inputs.saturating_sub(2);
        let extra_input_bufs = (0..extra_count).map(|_| Vec::new()).collect();
        let output_bufs = (0..dsp.num_outputs).map(|_| Vec::new()).collect();
        Self { dsp, info, sample_rate: 44100, extra_input_bufs, output_bufs }
    }

    /// Compile from source and wrap as a Machine.
    pub fn from_source(name: &str, source: &str) -> Result<Self, String> {
        let dsp = crate::compiler::compile(name, source, None)?;
        Ok(Self::new(dsp))
    }

    /// Compile from a .dsp file and wrap as a Machine.
    pub fn from_file(name: &str, path: &str) -> Result<Self, String> {
        let dsp = crate::compiler::compile_file(name, path, None)?;
        Ok(Self::new(dsp))
    }
}

impl AudioStream for FaustMachine {
    fn channel_config(&self) -> ChannelConfig {
        ChannelConfig {
            inputs: self.dsp.num_inputs as u16,
            outputs: self.dsp.num_outputs as u16,
        }
    }

    fn render(&mut self, output: &mut AudioBuffer) {
        let frames = output.frames() as usize;
        let buf_channels = output.channels() as usize;

        // Ensure extra input buffers are zeroed and large enough
        for buf in &mut self.extra_input_bufs {
            buf.resize(frames, 0.0);
            buf.iter_mut().for_each(|s| *s = 0.0);
        }

        // Build input pointers: first from AudioBuffer, rest from zero-filled extras
        let input_ptrs: Vec<*const f32> = (0..self.dsp.num_inputs)
            .map(|i| {
                if i < buf_channels {
                    output.channel(i as u16).as_ptr()
                } else {
                    self.extra_input_bufs[i - buf_channels].as_ptr()
                }
            })
            .collect();

        // Ensure scratch output buffers are large enough
        for buf in &mut self.output_bufs {
            buf.resize(frames, 0.0);
            buf.iter_mut().for_each(|s| *s = 0.0);
        }

        let mut output_ptrs: Vec<*mut f32> = self.output_bufs.iter_mut()
            .map(|b| b.as_mut_ptr())
            .collect();

        unsafe {
            ffi::computeCDSPInstance(
                self.dsp.instance(),
                frames as i32,
                input_ptrs.as_ptr(),
                output_ptrs.as_mut_ptr(),
            );
        }

        // Copy Faust output back into AudioBuffer channels
        for (i, buf) in self.output_bufs.iter().enumerate() {
            if i < buf_channels {
                let ch = output.channel_mut(i as u16);
                ch[..frames].copy_from_slice(&buf[..frames]);
            }
        }
    }
}

impl Machine for FaustMachine {
    fn info(&self) -> &MachineInfo { &self.info }

    fn init(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        unsafe { ffi::initCDSPInstance(self.dsp.instance(), sample_rate as i32); }
    }

    fn tick(&mut self) {}

    fn stop(&mut self) {
        unsafe { ffi::instanceClearCDSPInstance(self.dsp.instance()); }
    }

    fn set_param(&mut self, param: u16, value: i32) {
        if let Some(faust_param) = self.dsp.params().get(param as usize) {
            let f = ui_visitor::scaled_to_f32(faust_param, value);
            unsafe { *faust_param.zone = f; }
        }
    }
}
