//! Amiga-style one-pole RC low-pass filter.
//!
//! The Amiga's audio hardware applied a ~4.4 kHz RC filter to all output,
//! giving MOD files their characteristic warm sound.

use core::f32::consts::TAU;

use mb_ir::{AudioBuffer, AudioStream, ChannelConfig};
use crate::machine::{Machine, MachineInfo, MachineType, ParamInfo};

const DEFAULT_CUTOFF: i32 = 4410;

static PARAMS: &[ParamInfo] = &[ParamInfo {
    id: 0,
    name: "Cutoff",
    min: 1000,
    max: 22050,
    default: DEFAULT_CUTOFF,
    no_value: 0,
}];

static INFO: MachineInfo = MachineInfo {
    name: "Amiga Filter",
    short_name: "AFilter",
    author: "masterblaster",
    machine_type: MachineType::Effect,
    params: PARAMS,
};

/// One-pole RC low-pass filter: `y = y_prev + alpha * (x - y_prev)`.
pub struct AmigaFilter {
    prev_left: f32,
    prev_right: f32,
    alpha: f32,
    cutoff_hz: f32,
    sample_rate: u32,
}

impl AmigaFilter {
    pub fn new() -> Self {
        Self {
            prev_left: 0.0,
            prev_right: 0.0,
            alpha: 0.0,
            cutoff_hz: DEFAULT_CUTOFF as f32,
            sample_rate: 44100,
        }
    }

    fn recompute_alpha(&mut self) {
        self.alpha = TAU * self.cutoff_hz / self.sample_rate as f32;
    }
}

impl AudioStream for AmigaFilter {
    fn channel_config(&self) -> ChannelConfig {
        ChannelConfig { inputs: 2, outputs: 2 }
    }

    fn render(&mut self, output: &mut AudioBuffer) {
        let alpha = self.alpha;
        let frames = output.frames() as usize;

        // Process left channel
        let left = output.channel_mut(0);
        let mut prev = self.prev_left;
        for i in 0..frames {
            prev += alpha * (left[i] - prev);
            left[i] = prev;
        }
        self.prev_left = prev;

        // Process right channel (if present)
        if output.channels() >= 2 {
            let right = output.channel_mut(1);
            let mut prev = self.prev_right;
            for i in 0..frames {
                prev += alpha * (right[i] - prev);
                right[i] = prev;
            }
            self.prev_right = prev;
        }
    }
}

impl Machine for AmigaFilter {
    fn info(&self) -> &MachineInfo {
        &INFO
    }

    fn init(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        self.recompute_alpha();
    }

    fn tick(&mut self) {}

    fn stop(&mut self) {
        self.prev_left = 0.0;
        self.prev_right = 0.0;
    }

    fn set_param(&mut self, param: u16, value: i32) {
        if param == 0 {
            self.cutoff_hz = (value as f32).clamp(1000.0, 22050.0);
            self.recompute_alpha();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_filter(cutoff: i32, sr: u32) -> AmigaFilter {
        let mut f = AmigaFilter::new();
        f.set_param(0, cutoff);
        f.init(sr);
        f
    }

    #[test]
    fn alpha_at_default_cutoff() {
        let f = init_filter(DEFAULT_CUTOFF, 44100);
        let expected = TAU * 4410.0 / 44100.0;
        assert!((f.alpha - expected).abs() < 1e-6);
    }

    #[test]
    fn attenuates_high_frequency_content() {
        let mut f = init_filter(DEFAULT_CUTOFF, 44100);
        let n = 200;
        let mut buf = AudioBuffer::new(2, n);
        for i in 0..n as usize {
            let v = if i % 2 == 0 { 1.0 } else { -1.0 };
            buf.channel_mut(0)[i] = v;
            buf.channel_mut(1)[i] = v;
        }

        f.render(&mut buf);

        let peak: f32 = buf.channel(0).iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(peak < 0.95, "peak should be attenuated, got {}", peak);
    }

    #[test]
    fn passes_low_frequency_content() {
        let mut f = init_filter(DEFAULT_CUTOFF, 44100);
        let n = 100;
        let mut buf = AudioBuffer::new(2, n);
        for i in 0..n as usize {
            buf.channel_mut(0)[i] = 0.5;
            buf.channel_mut(1)[i] = 0.5;
        }

        f.render(&mut buf);

        let last = buf.channel(0)[n as usize - 1];
        assert!(
            (last - 0.5).abs() < 0.01,
            "DC should pass through, got {}",
            last
        );
    }

    #[test]
    fn stop_resets_state() {
        let mut f = init_filter(DEFAULT_CUTOFF, 44100);
        let mut buf = AudioBuffer::new(2, 10);
        for i in 0..10 {
            buf.channel_mut(0)[i] = 1.0;
        }
        f.render(&mut buf);
        assert!(f.prev_left != 0.0);

        f.stop();
        assert_eq!(f.prev_left, 0.0);
        assert_eq!(f.prev_right, 0.0);
    }
}
