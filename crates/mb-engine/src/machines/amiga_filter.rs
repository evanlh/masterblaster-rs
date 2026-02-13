//! Amiga-style one-pole RC low-pass filter.
//!
//! The Amiga's audio hardware applied a ~4.4 kHz RC filter to all output,
//! giving MOD files their characteristic warm sound.

use core::f32::consts::TAU;

use crate::machine::{Machine, MachineInfo, MachineType, ParamInfo, WorkMode};

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

impl Machine for AmigaFilter {
    fn info(&self) -> &MachineInfo {
        &INFO
    }

    fn init(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        self.recompute_alpha();
    }

    fn tick(&mut self) {}

    fn work(&mut self, buffer: &mut [f32], mode: WorkMode) -> bool {
        if mode == WorkMode::NoIO || mode == WorkMode::Write {
            return false;
        }
        let alpha = self.alpha;
        let mut prev_l = self.prev_left;
        let mut prev_r = self.prev_right;

        for pair in buffer.chunks_exact_mut(2) {
            prev_l += alpha * (pair[0] - prev_l);
            prev_r += alpha * (pair[1] - prev_r);
            pair[0] = prev_l;
            pair[1] = prev_r;
        }

        self.prev_left = prev_l;
        self.prev_right = prev_r;
        true
    }

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
        // Generate a high-frequency square wave (alternating +1/-1)
        let mut buf: Vec<f32> = (0..200)
            .flat_map(|i| {
                let v = if i % 2 == 0 { 1.0 } else { -1.0 };
                [v, v]
            })
            .collect();

        f.work(&mut buf, WorkMode::ReadWrite);

        // After filtering, peak amplitude should be reduced
        let peak: f32 = buf.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(peak < 0.95, "peak should be attenuated, got {}", peak);
    }

    #[test]
    fn passes_low_frequency_content() {
        let mut f = init_filter(DEFAULT_CUTOFF, 44100);
        // DC signal should pass through nearly unchanged
        let mut buf = vec![0.5f32; 200];
        f.work(&mut buf, WorkMode::ReadWrite);

        // After settling, values should be close to 0.5
        let last = buf[buf.len() - 2];
        assert!(
            (last - 0.5).abs() < 0.01,
            "DC should pass through, got {}",
            last
        );
    }

    #[test]
    fn stop_resets_state() {
        let mut f = init_filter(DEFAULT_CUTOFF, 44100);
        let mut buf = vec![1.0; 20];
        f.work(&mut buf, WorkMode::ReadWrite);
        assert!(f.prev_left != 0.0);

        f.stop();
        assert_eq!(f.prev_left, 0.0);
        assert_eq!(f.prev_right, 0.0);
    }
}
