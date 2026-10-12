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

    // --- WahMan Pro tests ---

    #[test]
    fn wahpro_compiles() {
        let dsp = load_dsp("wahpro", "wahpro.dsp");
        assert_eq!(dsp.num_inputs, 2, "wahpro should be stereo in");
        assert_eq!(dsp.num_outputs, 2, "wahpro should be stereo out");
        assert_eq!(dsp.params.len(), 8, "wahpro should have 8 params");
    }

    #[test]
    fn wahpro_filters_signal() {
        let mut machine = FaustMachine::new(load_dsp("wahpro", "wahpro.dsp"));
        machine.init(44100);

        // Feed a broadband impulse and check output is non-zero
        let mut buf = AudioBuffer::new(2, 256);
        buf.channel_mut(0)[0] = 1.0;
        buf.channel_mut(1)[0] = 1.0;
        machine.render(&mut buf);

        let energy: f32 = buf.channel(0).iter().map(|s| s * s).sum();
        assert!(energy > 0.0001, "wahpro should produce output, energy={energy}");
    }

    // --- WahMan Pro 2 tests ---

    #[test]
    fn wahpro2_compiles() {
        let dsp = load_dsp("wahpro2", "wahpro2.dsp");
        assert_eq!(dsp.num_inputs, 2, "wahpro2 should be stereo in");
        assert_eq!(dsp.num_outputs, 2, "wahpro2 should be stereo out");
        assert_eq!(dsp.params.len(), 8, "wahpro2 should have 8 params");
    }

    #[test]
    fn wahpro2_filters_signal() {
        let mut machine = FaustMachine::new(load_dsp("wahpro2", "wahpro2.dsp"));
        machine.init(44100);

        let mut buf = AudioBuffer::new(2, 256);
        buf.channel_mut(0)[0] = 1.0;
        buf.channel_mut(1)[0] = 1.0;
        machine.render(&mut buf);

        let energy: f32 = buf.channel(0).iter().map(|s| s * s).sum();
        assert!(energy > 0.0001, "wahpro2 should produce output, energy={energy}");
    }

    #[test]
    fn wahpro2_lp_attenuates_highs() {
        let mut machine = FaustMachine::new(load_dsp("wahpro2", "wahpro2.dsp"));
        machine.init(44100);

        // Set Type=0 (LP), low cutoff, no LFO
        machine.set_param(2, 0);      // Filter = 0 (LP)
        machine.set_param(0, 6553);   // Cutoff ~20/240 (low)
        machine.set_param(4, 0);      // LFO mod = 0

        // Generate 10kHz sine
        let frames = 1024_u16;
        let mut buf = AudioBuffer::new(2, frames);
        for i in 0..frames as usize {
            let s = (2.0 * std::f32::consts::PI * 10000.0 * i as f32 / 44100.0).sin();
            buf.channel_mut(0)[i] = s;
            buf.channel_mut(1)[i] = s;
        }
        machine.render(&mut buf);

        let energy: f32 = buf.channel(0)[512..].iter().map(|s| s * s).sum();
        let rms = (energy / 512.0).sqrt();
        // WahPro2 has 2x output gain + 1/sqrt(Q) normalization, so attenuation is less dramatic
        assert!(rms < 1.0, "LP filter should attenuate 10kHz, RMS={rms}");
    }

    // --- Parameter ordering + range validation ---

    struct ExpectedParam {
        label: &'static str,
        min: f32,
        max: f32,
        init: f32,
    }

    fn assert_params_match(dsp_name: &str, dsp_file: &str, expected: &[ExpectedParam]) {
        let dsp = load_dsp(dsp_name, dsp_file);
        assert_eq!(dsp.params.len(), expected.len(),
            "{dsp_name}: param count mismatch");
        for (i, (got, exp)) in dsp.params.iter().zip(expected).enumerate() {
            assert_eq!(got.label, exp.label,
                "{dsp_name} param[{i}]: label mismatch");
            assert!((got.min - exp.min).abs() < 0.01,
                "{dsp_name} param[{i}] ({}) min: expected {}, got {}", exp.label, exp.min, got.min);
            assert!((got.max - exp.max).abs() < 0.01,
                "{dsp_name} param[{i}] ({}) max: expected {}, got {}", exp.label, exp.max, got.max);
            assert!((got.init - exp.init).abs() < 0.01,
                "{dsp_name} param[{i}] ({}) init: expected {}, got {}", exp.label, exp.init, got.init);
        }
    }

    // Faust sorts params alphabetically by label unless labels carry `[N]`
    // order metadata. BMX automation indexes params by the C++ pParameters[]
    // order, so DSPs mirroring Buzz machines must prefix labels with `[N]`.

    #[test]
    fn wahpro2_param_ordering_and_ranges() {
        assert_params_match("wahpro2", "wahpro2.dsp", &[
            ExpectedParam { label: "Cutoff",    min: 0.0, max: 240.0, init: 120.0 },
            ExpectedParam { label: "Resonance", min: 0.0, max: 240.0, init: 120.0 },
            ExpectedParam { label: "Filter",    min: 0.0, max: 7.0,   init: 6.0 },
            ExpectedParam { label: "LFO rate",  min: 0.0, max: 254.0, init: 30.0 },
            ExpectedParam { label: "LFO mod",   min: 0.0, max: 240.0, init: 60.0 },
            ExpectedParam { label: "LFO Shape", min: 0.0, max: 16.0,  init: 0.0 },
            ExpectedParam { label: "Inertia",   min: 0.0, max: 60.0,  init: 10.0 },
            ExpectedParam { label: "Set phase", min: 0.0, max: 127.0, init: 0.0 },
        ]);
    }

    #[test]
    fn wahpro_param_ordering_and_ranges() {
        assert_params_match("wahpro", "wahpro.dsp", &[
            ExpectedParam { label: "Cutoff",    min: 0.0, max: 240.0, init: 120.0 },
            ExpectedParam { label: "Resonance", min: 0.0, max: 240.0, init: 120.0 },
            ExpectedParam { label: "Filter",    min: 0.0, max: 4.0,   init: 1.0 },
            ExpectedParam { label: "LFO rate",  min: 0.0, max: 240.0, init: 30.0 },
            ExpectedParam { label: "LFO mod",   min: 0.0, max: 240.0, init: 60.0 },
            ExpectedParam { label: "LFO Shape", min: 0.0, max: 8.0,   init: 1.0 },
            ExpectedParam { label: "Inertia",   min: 0.0, max: 60.0,  init: 10.0 },
            ExpectedParam { label: "Set phase", min: 0.0, max: 127.0, init: 0.0 },
        ]);
    }

    #[test]
    fn filter2_param_ordering_and_ranges() {
        assert_params_match("filter2", "filter2.dsp", &[
            ExpectedParam { label: "Type",      min: 0.0, max: 3.0,   init: 0.0 },
            ExpectedParam { label: "Cutoff",    min: 0.0, max: 127.0, init: 64.0 },
            ExpectedParam { label: "Resonance", min: 0.0, max: 127.0, init: 0.0 },
        ]);
    }

    #[test]
    fn reverb2_param_ordering_and_ranges() {
        assert_params_match("reverb2", "reverb2.dsp", &[
            ExpectedParam { label: "Dry Out",   min: 0.0, max: 127.0, init: 127.0 },
            ExpectedParam { label: "Rev Out",   min: 0.0, max: 127.0, init: 80.0 },
            ExpectedParam { label: "ER Out",    min: 0.0, max: 127.0, init: 64.0 },
            ExpectedParam { label: "Rev Time",  min: 0.0, max: 127.0, init: 64.0 },
            ExpectedParam { label: "Pre-Delay", min: 0.0, max: 127.0, init: 10.0 },
        ]);
    }

    #[test]
    fn freeverb_param_ordering_and_ranges() {
        assert_params_match("freeverb", "reverb.dsp", &[
            ExpectedParam { label: "Damping",   min: 0.0, max: 1.0, init: 0.5 },
            ExpectedParam { label: "Room Size", min: 0.0, max: 1.0, init: 0.5 },
            ExpectedParam { label: "Wet",       min: 0.0, max: 1.0, init: 0.33 },
        ]);
    }

    /// Set each BMX param index to max and check it lands on the
    /// identically-labelled Faust param (C++ pParameters[] order).
    fn assert_set_param_routes(dsp_name: &str, dsp_file: &str, bmx_order: &[&str]) {
        let dsp = load_dsp(dsp_name, dsp_file);
        let zones: Vec<_> = bmx_order.iter()
            .map(|label| (find_param(dsp.params(), label).zone, find_param(dsp.params(), label).max))
            .collect();
        let mut machine = FaustMachine::new(dsp);
        for (i, (label, (zone, max))) in bmx_order.iter().zip(&zones).enumerate() {
            machine.set_param(i as u16, 65535);
            let got = unsafe { **zone };
            assert!((got - max).abs() < 0.01,
                "{dsp_name}: set_param({i}) should set '{label}' to {max}, got {got}");
        }
    }

    #[test]
    fn wahpro2_set_param_follows_bmx_order() {
        assert_set_param_routes("wahpro2", "wahpro2.dsp", &[
            "Cutoff", "Resonance", "Filter", "LFO rate", "LFO mod", "LFO Shape", "Inertia", "Set phase",
        ]);
    }

    #[test]
    fn wahpro_set_param_follows_bmx_order() {
        assert_set_param_routes("wahpro", "wahpro.dsp", &[
            "Cutoff", "Resonance", "Filter", "LFO rate", "LFO mod", "LFO Shape", "Inertia", "Set phase",
        ]);
    }

    #[test]
    fn filter2_set_param_follows_bmx_order() {
        assert_set_param_routes("filter2", "filter2.dsp", &["Type", "Cutoff", "Resonance"]);
    }

    #[test]
    fn reverb2_set_param_follows_bmx_order() {
        assert_set_param_routes("reverb2", "reverb2.dsp", &[
            "Dry Out", "Rev Out", "ER Out", "Rev Time", "Pre-Delay",
        ]);
    }

    // --- Round-trip scaling tests ---

    fn round_trip_f32(param: &crate::ui_visitor::FaustParam, raw: f32) -> f32 {
        // Simulate BMX pipeline: raw → scale to 0..65535 → scaled_to_f32
        let range = param.max - param.min;
        let scaled = if range > 0.0 { ((raw - param.min) / range * 65535.0) as i32 } else { 0 };
        crate::ui_visitor::scaled_to_f32(param, scaled)
    }

    #[test]
    fn wahpro2_defaults_round_trip() {
        let dsp = load_dsp("wahpro2", "wahpro2.dsp");
        for p in &dsp.params {
            let result = round_trip_f32(p, p.init);
            assert!((result - p.init).abs() < 0.01,
                "{}: init {}, round-tripped to {result}", p.label, p.init);
        }
    }

    #[test]
    fn wahpro2_boundary_values_round_trip() {
        let dsp = load_dsp("wahpro2", "wahpro2.dsp");
        for p in &dsp.params {
            let at_min = crate::ui_visitor::scaled_to_f32(p, 0);
            assert!((at_min - p.min).abs() < 0.01,
                "{}: min round-trip expected {}, got {at_min}", p.label, p.min);
            let at_max = crate::ui_visitor::scaled_to_f32(p, 65535);
            assert!((at_max - p.max).abs() < 0.01,
                "{}: max round-trip expected {}, got {at_max}", p.label, p.max);
        }
    }

    fn find_param<'a>(params: &'a [crate::ui_visitor::FaustParam], label: &str) -> &'a crate::ui_visitor::FaustParam {
        params.iter().find(|p| p.label == label).unwrap_or_else(|| panic!("param '{label}' not found"))
    }

    #[test]
    fn wahpro2_integer_params_quantize_cleanly() {
        let dsp = load_dsp("wahpro2", "wahpro2.dsp");
        // Filter type (0-7) should quantize to integers
        let filter_param = find_param(&dsp.params, "Filter");
        for expected in 0..=7 {
            let result = round_trip_f32(filter_param, expected as f32);
            assert!((result - expected as f32).abs() < 0.5,
                "Filter type {expected} round-trips to {result}");
        }
        // LFO Shape (0-16) should quantize to integers
        let lfo_shape = find_param(&dsp.params, "LFO Shape");
        for expected in 0..=16 {
            let result = round_trip_f32(lfo_shape, expected as f32);
            assert!((result - expected as f32).abs() < 0.5,
                "LFO Shape {expected} round-trips to {result}");
        }
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
    fn registry_resolves_wahpro() {
        assert!(crate::create_faust_machine("FSM WahPro").is_some());
    }

    #[test]
    fn registry_resolves_wahpro2() {
        assert!(crate::create_faust_machine("FSM WahPro2").is_some());
    }

    #[test]
    fn registry_returns_none_for_unknown() {
        assert!(crate::create_faust_machine("Unknown Machine XYZ").is_none());
    }
}
