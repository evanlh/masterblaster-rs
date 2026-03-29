//! Faust JIT integration tests.

#[cfg(test)]
mod tests {
    use crate::compiler;
    use crate::faust_machine::FaustMachine;
    use mb_ir::{AudioBuffer, AudioStream};
    use mb_engine::machine::Machine;

    #[test]
    fn compile_passthrough_smoke() {
        let dsp = compiler::compile("test", "process = _;", None).unwrap();
        assert_eq!(dsp.num_inputs, 1);
        assert_eq!(dsp.num_outputs, 1);
    }

    #[test]
    fn compile_stereo_passthrough() {
        let dsp = compiler::compile("test", "process = _, _;", None).unwrap();
        assert_eq!(dsp.num_inputs, 2);
        assert_eq!(dsp.num_outputs, 2);
    }

    #[test]
    fn compile_error_returns_err() {
        let result = compiler::compile("test", "not valid faust code!!!!", None);
        match result {
            Ok(_) => panic!("expected compile error"),
            Err(msg) => assert!(!msg.is_empty(), "error message should be non-empty"),
        }
    }

    #[test]
    fn param_discovery_finds_sliders() {
        let source = r#"
            gain = hslider("Gain", 0.5, 0.0, 1.0, 0.01);
            freq = hslider("Freq", 440.0, 20.0, 20000.0, 1.0);
            process = *(gain), *(freq/20000.0);
        "#;
        let dsp = compiler::compile("test", source, None).unwrap();
        assert_eq!(dsp.params.len(), 2);
        assert!(dsp.params.iter().any(|p| p.label == "Gain"));
        assert!(dsp.params.iter().any(|p| p.label == "Freq"));
    }

    #[test]
    fn passthrough_renders_identity() {
        let mut machine = FaustMachine::from_source("test", "process = _, _;").unwrap();
        machine.init(44100);

        let mut buf = AudioBuffer::new(2, 4);
        buf.channel_mut(0).copy_from_slice(&[0.1, 0.2, 0.3, 0.4]);
        buf.channel_mut(1).copy_from_slice(&[0.5, 0.6, 0.7, 0.8]);

        machine.render(&mut buf);

        assert_eq!(buf.channel(0), &[0.1, 0.2, 0.3, 0.4]);
        assert_eq!(buf.channel(1), &[0.5, 0.6, 0.7, 0.8]);
    }

    #[test]
    fn gain_halves_amplitude() {
        let mut machine = FaustMachine::from_source("test", "process = *(0.5), *(0.5);").unwrap();
        machine.init(44100);

        let mut buf = AudioBuffer::new(2, 4);
        buf.channel_mut(0).copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        buf.channel_mut(1).copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);

        machine.render(&mut buf);

        for &v in buf.channel(0) {
            assert!((v - 0.5).abs() < 0.001, "expected ~0.5, got {v}");
        }
    }

    #[test]
    fn reverb_produces_tail() {
        let source = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../faust/reverb.dsp")
        ).unwrap();
        let dsp = compiler::compile("reverb", &source, None).unwrap();

        let mut machine = FaustMachine::new(dsp);
        machine.init(44100);

        // Render an impulse followed by ~0.5s of silence (freeverb has ~1557-sample combs)
        let block = 256_u16;
        let mut buf = AudioBuffer::new(2, block);
        buf.channel_mut(0)[0] = 1.0;
        buf.channel_mut(1)[0] = 1.0;
        machine.render(&mut buf);

        // The dry path should pass through immediately
        assert!(buf.channel(0)[0].abs() > 0.5, "dry signal should pass, got {}", buf.channel(0)[0]);

        // Render more blocks to let reverb tail emerge
        let mut tail_energy: f32 = 0.0;
        for _ in 0..100 {
            let mut tail = AudioBuffer::new(2, block);
            machine.render(&mut tail);
            tail_energy += tail.channel(0).iter().map(|s| s * s).sum::<f32>();
        }
        assert!(tail_energy > 0.0001, "reverb should produce a tail, energy={tail_energy}");
    }

    #[test]
    fn reverb_has_three_params() {
        let source = std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../faust/reverb.dsp")
        ).unwrap();
        let dsp = compiler::compile("reverb", &source, None).unwrap();
        assert_eq!(dsp.params.len(), 3);
        let labels: Vec<&str> = dsp.params.iter().map(|p| p.label.as_str()).collect();
        assert!(labels.contains(&"Room Size"));
        assert!(labels.contains(&"Damping"));
        assert!(labels.contains(&"Wet"));
    }

    #[test]
    fn stop_clears_state() {
        let mut machine = FaustMachine::from_source("test", "process = _, _;").unwrap();
        machine.init(44100);
        machine.stop();
        // Should not panic, just clears internal delay lines
    }

    // --- Filter 2 tests ---

    fn load_dsp(name: &str, path: &str) -> crate::compiler::CompiledDsp {
        let source = std::fs::read_to_string(
            format!("{}/../../faust/{path}", env!("CARGO_MANIFEST_DIR"))
        ).unwrap();
        compiler::compile(name, &source, None).unwrap()
    }

    #[test]
    fn filter2_compiles() {
        let dsp = load_dsp("filter2", "filter2.dsp");
        assert_eq!(dsp.num_inputs, 2, "filter2 should be stereo in");
        assert_eq!(dsp.num_outputs, 2, "filter2 should be stereo out");
        assert_eq!(dsp.params.len(), 3, "filter2 should have 3 params (Type, Cutoff, Resonance)");
    }

    #[test]
    fn filter2_lowpass_attenuates_high_freq() {
        let mut machine = FaustMachine::from_source("filter2",
            &std::fs::read_to_string(
                format!("{}/../../faust/filter2.dsp", env!("CARGO_MANIFEST_DIR"))
            ).unwrap()
        ).unwrap();
        machine.init(44100);

        // Set Type=0 (LP), Cutoff=20 (low cutoff ~50Hz), Resonance=0
        machine.set_param(0, 0);      // Type = LP
        machine.set_param(1, 10240);  // Cutoff low (~20/127 * 65535)
        machine.set_param(2, 0);      // Resonance = 0

        // Generate high-frequency sine (10kHz) as input
        let frames = 1024_u16;
        let mut buf = AudioBuffer::new(2, frames);
        for i in 0..frames as usize {
            let sample = (2.0 * std::f32::consts::PI * 10000.0 * i as f32 / 44100.0).sin();
            buf.channel_mut(0)[i] = sample;
            buf.channel_mut(1)[i] = sample;
        }

        machine.render(&mut buf);

        // High frequency should be significantly attenuated by low-pass
        let energy: f32 = buf.channel(0)[512..].iter().map(|s| s * s).sum();
        let rms = (energy / 512.0).sqrt();
        assert!(rms < 0.3, "LP filter should attenuate 10kHz, RMS={rms}");
    }

    // --- Reverb 2 tests ---

    #[test]
    fn reverb2_compiles() {
        let dsp = load_dsp("reverb2", "reverb2.dsp");
        assert_eq!(dsp.num_inputs, 2, "reverb2 should be stereo in");
        assert_eq!(dsp.num_outputs, 2, "reverb2 should be stereo out");
        assert_eq!(dsp.params.len(), 5, "reverb2 should have 5 params");
        let labels: Vec<&str> = dsp.params.iter().map(|p| p.label.as_str()).collect();
        assert!(labels.contains(&"Dry Out"));
        assert!(labels.contains(&"Rev Out"));
        assert!(labels.contains(&"ER Out"));
        assert!(labels.contains(&"Rev Time"));
        assert!(labels.contains(&"Pre-Delay"));
    }

    #[test]
    fn reverb2_produces_tail() {
        let mut machine = FaustMachine::from_source("reverb2",
            &std::fs::read_to_string(
                format!("{}/../../faust/reverb2.dsp", env!("CARGO_MANIFEST_DIR"))
            ).unwrap()
        ).unwrap();
        machine.init(44100);

        // Render an impulse
        let block = 256_u16;
        let mut buf = AudioBuffer::new(2, block);
        buf.channel_mut(0)[0] = 1.0;
        buf.channel_mut(1)[0] = 1.0;
        machine.render(&mut buf);

        // Dry signal should pass through
        assert!(buf.channel(0)[0].abs() > 0.1, "dry signal should pass, got {}", buf.channel(0)[0]);

        // Render more blocks to verify reverb tail
        let mut tail_energy: f32 = 0.0;
        for _ in 0..100 {
            let mut tail = AudioBuffer::new(2, block);
            machine.render(&mut tail);
            tail_energy += tail.channel(0).iter().map(|s| s * s).sum::<f32>();
        }
        assert!(tail_energy > 0.0001, "reverb2 should produce a tail, energy={tail_energy}");
    }

    // --- Registry tests ---

    #[test]
    fn registry_resolves_filter2() {
        assert!(crate::create_faust_machine("Jeskola Filter 2").is_some());
    }

    #[test]
    fn registry_resolves_reverb2() {
        assert!(crate::create_faust_machine("Jeskola Reverb 2").is_some());
    }

    #[test]
    fn registry_resolves_freeverb() {
        assert!(crate::create_faust_machine("Jeskola Freeverb").is_some());
    }

    #[test]
    fn registry_returns_none_for_unknown() {
        assert!(crate::create_faust_machine("Unknown Machine XYZ").is_none());
    }
}
