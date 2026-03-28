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
}
