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
        assert_eq!(dsp.params.len(), 6, "reverb2 should have 6 params");
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

    #[test]
    fn wahpro2_bandpeak_dc_gain_matches_measured() {
        // Filter 2 (single peaking EQ) passes DC at 1.2 / (7 - res/60) = 0.24 at res 120,
        // as measured from the original's per-block filter fits.
        let mut machine = FaustMachine::new(load_dsp("wahpro2", "wahpro2.dsp"));
        machine.init(44100);
        machine.set_param(2, 18724); // Filter = 2
        machine.set_param(4, 0);     // LFO mod = 0
        let mut buf = AudioBuffer::new(2, 8192);
        buf.channel_mut(0).fill(0.5);
        buf.channel_mut(1).fill(0.5);
        machine.render(&mut buf);
        let last = buf.channel(0)[8191];
        assert!((last - 0.12).abs() < 0.002, "DC gain should be 0.24, got {}", last / 0.5);
    }

    #[test]
    fn wahpro2_silent_block_outputs_silence() {
        // The original returns false from Work on silent input, so its ringing tail
        // is cut at the next all-silent render block.
        let mut machine = FaustMachine::new(load_dsp("wahpro2", "wahpro2.dsp"));
        machine.init(44100);
        let mut loud = AudioBuffer::new(2, 256);
        loud.channel_mut(0)[0] = 1.0;
        loud.channel_mut(1)[0] = 1.0;
        machine.render(&mut loud);
        assert!(loud.channel(0).iter().any(|s| *s != 0.0), "impulse block should ring");
        let mut silent = AudioBuffer::new(2, 256);
        machine.render(&mut silent);
        assert!(silent.channel(0).iter().all(|s| *s == 0.0), "silent block should be cut");
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

    const WAHMANPRO_ORDER: [&str; 8] = [
        "Cutoff", "Resonance", "Filter", "LFO rate", "LFO mod", "LFO Shape", "Inertia", "Set phase",
    ];

    #[test]
    fn wahmanpro_param_ordering_and_ranges() {
        // Set phase: Buzz default is 255 (no-value, outside 0..127); slider init 0.
        assert_params_match("wahmanpro", "wahmanpro.dsp", &[
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
    fn wahmanpro_set_param_follows_bmx_order() {
        assert_set_param_routes("wahmanpro", "wahmanpro.dsp", &WAHMANPRO_ORDER);
    }

    #[test]
    fn wahmanpro_is_registered_stereo() {
        let dsp = load_dsp("wahmanpro", "wahmanpro.dsp");
        assert_eq!((dsp.num_inputs, dsp.num_outputs), (2, 2));
        assert!(crate::registry::create_faust_machine("FSM WahManPro").is_some());
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
            // Ranges and defaults from the BMX PARA section (no C++ source available)
            ExpectedParam { label: "Dry Out",     min: 0.0,   max: 4096.0,  init: 4096.0 },
            ExpectedParam { label: "Rev Out",     min: 0.0,   max: 4096.0,  init: 768.0 },
            ExpectedParam { label: "ER Out",      min: 0.0,   max: 4096.0,  init: 0.0 },
            ExpectedParam { label: "Rev Time",    min: 1.0,   max: 64.0,    init: 10.0 },
            ExpectedParam { label: "Pre-Delay",   min: 0.0,   max: 100.0,   init: 0.0 },
            ExpectedParam { label: "High Cutoff", min: 100.0, max: 16384.0, init: 5000.0 },
        ]);
    }

    #[test]
    fn freeverb_param_ordering_and_ranges() {
        assert_params_match("freeverb", "reverb.dsp", &[
            ExpectedParam { label: "Rev Time",  min: 0.0, max: 127.0, init: 40.0 },
            ExpectedParam { label: "HF Damp",   min: 0.0, max: 127.0, init: 50.0 },
            ExpectedParam { label: "Pre Delay", min: 0.0, max: 127.0, init: 0.0 },
            ExpectedParam { label: "Low Cut",   min: 0.0, max: 127.0, init: 0.0 },
            ExpectedParam { label: "High Cut",  min: 0.0, max: 127.0, init: 127.0 },
            ExpectedParam { label: "Rev Out",   min: 0.0, max: 127.0, init: 80.0 },
            ExpectedParam { label: "Dry Out",   min: 0.0, max: 127.0, init: 127.0 },
        ]);
    }

    #[test]
    fn fsm_chorus_param_ordering_and_ranges() {
        assert_params_match("fsm_chorus", "fsm_chorus.dsp", &[
            ExpectedParam { label: "Dry out",       min: 0.0, max: 240.0, init: 240.0 },
            ExpectedParam { label: "Minimum delay", min: 1.0, max: 100.0, init: 20.0 },
            ExpectedParam { label: "Modulation",    min: 1.0, max: 100.0, init: 10.0 },
            ExpectedParam { label: "LFO rate",      min: 0.0, max: 240.0, init: 10.0 },
            ExpectedParam { label: "Wet out",       min: 0.0, max: 240.0, init: 240.0 },
            ExpectedParam { label: "Feedback",      min: 0.0, max: 128.0, init: 64.0 },
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

    const SCRAPMAN_LABELS: [&str; 12] = [
        "Dry out", "Feedback", "Fullness", "Richness", "Density", "Scattering",
        "Fatness", "Attack", "Sustain", "Release", "Wet out", "Dummy",
    ];

    #[test]
    fn scrapman_param_ordering_and_ranges() {
        let raw = [(0.0, 240.0, 240.0), (0.0, 99.0, 0.0), (0.0, 100.0, 0.0), (4.0, 24.0, 12.0),
            (0.0, 64.0, 48.0), (0.0, 64.0, 48.0), (0.0, 64.0, 16.0), (0.0, 240.0, 120.0),
            (0.0, 240.0, 32.0), (0.0, 240.0, 64.0), (0.0, 240.0, 240.0), (0.0, 240.0, 240.0)];
        let expected: Vec<ExpectedParam> = SCRAPMAN_LABELS.iter().zip(raw)
            .map(|(l, (min, max, init))| ExpectedParam { label: l, min, max, init }).collect();
        assert_params_match("scrapman", "scrapman.dsp", &expected);
    }

    #[test]
    fn scrapman_set_param_follows_bmx_order() {
        assert_set_param_routes("scrapman", "scrapman.dsp", &SCRAPMAN_LABELS);
    }

    #[test]
    fn scrapman_boundary_values_round_trip() {
        let dsp = load_dsp("scrapman", "scrapman.dsp");
        for p in &dsp.params {
            let at_max = crate::ui_visitor::scaled_to_f32(p, 65535);
            assert!((at_max - p.max).abs() < 0.01, "{}: max {}", p.label, at_max);
            let at_min = crate::ui_visitor::scaled_to_f32(p, 0);
            assert!((at_min - p.min).abs() < 0.01, "{}: min {}", p.label, at_min);
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
    fn lofi_param_ordering_and_ranges() {
        assert_params_match("lofi", "lofi.dsp", &[
            ExpectedParam { label: "Hold",       min: 0.0, max: 127.0, init: 0.0 },
            ExpectedParam { label: "Resolution", min: 0.0, max: 127.0, init: 0.0 },
            ExpectedParam { label: "Mode",       min: 0.0, max: 8.0,   init: 0.0 },
        ]);
    }

    #[test]
    fn lofi_set_param_follows_bmx_order() {
        assert_set_param_routes("lofi", "lofi.dsp", &["Hold", "Resolution", "Mode"]);
    }

    #[test]
    fn lofi_lo_mode_quantizes_toward_zero() {
        // Mode 7 (LO), Resolution 0 (step 128), Hold 0: output is the input truncated to 128 steps.
        let mut machine = FaustMachine::new(load_dsp("lofi", "lofi.dsp"));
        machine.init(44100);
        machine.set_param(2, (65535 * 7 / 8) as i32);
        let mut buf = AudioBuffer::new(2, 256);
        for i in 0..256 {
            buf.channel_mut(0)[i] = 0.1;
            buf.channel_mut(1)[i] = 0.1;
        }
        machine.render(&mut buf);
        let want = (0.1_f32 * 32768.0 / 128.0).trunc() * 128.0 / 32768.0;
        assert!((buf.channel(0)[100] - want).abs() < 1e-6, "got {}", buf.channel(0)[100]);
    }

    #[test]
    fn lofi_silent_block_outputs_silence() {
        let mut machine = FaustMachine::new(load_dsp("lofi", "lofi.dsp"));
        machine.init(44100);
        let mut buf = AudioBuffer::new(2, 256);
        machine.render(&mut buf);
        assert!(buf.channel(0).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn filter2_set_param_follows_bmx_order() {
        assert_set_param_routes("filter2", "filter2.dsp", &["Type", "Cutoff", "Resonance"]);
    }

    #[test]
    fn degrader_param_ordering_and_ranges() {
        assert_params_match("degrader", "degrader.dsp", &[
            ExpectedParam { label: "Smp. Rate", min: 1.0, max: 32.0, init: 1.0 },
            ExpectedParam { label: "Bit Depth", min: 2.0, max: 16.0, init: 16.0 },
        ]);
    }

    #[test]
    fn degrader_set_param_follows_bmx_order() {
        assert_set_param_routes("degrader", "degrader.dsp", &["Smp. Rate", "Bit Depth"]);
    }

    #[test]
    fn reverb2_set_param_follows_bmx_order() {
        assert_set_param_routes("reverb2", "reverb2.dsp", &[
            "Dry Out", "Rev Out", "ER Out", "Rev Time", "Pre-Delay", "High Cutoff",
        ]);
    }

    #[test]
    fn psycho01_param_ordering_and_ranges() {
        assert_params_match("psycho01", "psycho01.dsp", &[
            ExpectedParam { label: "Room", min: 0.0, max: 128.0, init: 128.0 },
        ]);
    }

    #[test]
    fn psycho01_set_param_follows_bmx_order() {
        assert_set_param_routes("psycho01", "psycho01.dsp", &["Room"]);
    }

    /// Render a constant mono 1.0 through Psycho01 at the given BMX value.
    fn render_psycho01(bmx_room: i32) -> AudioBuffer {
        let mut machine = FaustMachine::new(load_dsp("psycho01", "psycho01.dsp"));
        machine.init(44100);
        machine.set_param(0, bmx_room);
        let mut buf = AudioBuffer::new(2, 4);
        buf.channel_mut(0).fill(1.0);
        buf.channel_mut(1).fill(1.0);
        machine.render(&mut buf);
        buf
    }

    #[test]
    fn psycho01_room_zero_alternates_channels_per_sample() {
        let buf = render_psycho01(0);
        assert_eq!(buf.channel(0), &[0.0, 1.0, 0.0, 1.0]);
        assert_eq!(buf.channel(1), &[1.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn psycho01_room_max_is_centred() {
        let buf = render_psycho01(65535);
        assert!(buf.channel(0).iter().chain(buf.channel(1)).all(|&v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn freeverb_set_param_follows_bmx_order() {
        assert_set_param_routes("freeverb", "reverb.dsp", &[
            "Rev Time", "HF Damp", "Pre Delay", "Low Cut", "High Cut", "Rev Out", "Dry Out",
        ]);
    }

    // --- Automaton Compressor mkII ---

    const ACM2: (&str, &str) = ("automaton_compressor_mkii", "automaton_compressor_mkii.dsp");
    const ACM2_ORDER: [&str; 7] = ["Mode", "Channel", "Threshold", "Ratio", "Attack", "Release", "Gain"];

    #[test]
    fn automaton_compressor_mkii_param_ordering_and_ranges() {
        assert_params_match(ACM2.0, ACM2.1, &[
            ExpectedParam { label: "Mode",      min: 0.0, max: 2.0,     init: 0.0 },
            ExpectedParam { label: "Channel",   min: 1.0, max: 32.0,    init: 1.0 },
            ExpectedParam { label: "Threshold", min: 0.0, max: 65534.0, init: 47512.0 },
            ExpectedParam { label: "Ratio",     min: 0.0, max: 65534.0, init: 0.0 },
            ExpectedParam { label: "Attack",    min: 0.0, max: 65534.0, init: 12577.0 },
            ExpectedParam { label: "Release",   min: 0.0, max: 65534.0, init: 7452.0 },
            ExpectedParam { label: "Gain",      min: 0.0, max: 65534.0, init: 0.0 },
    // --- FSM PanzerDelay ---

    const PANZER_LABELS: [&str; 15] = [
        "Dry amp", "Dry L Pan", "Dry R Pan", "Delay unit", "Filter type", "Cutoff", "Resonance",
        "L Delay Len", "R Delay Len", "Tap Amp", "Wet L Pan", "Wet R Pan", "FB Amp",
        "FB PingPong", "FB Spread",
    ];

    #[test]
    fn panzerdelay_param_ordering_and_ranges() {
        let spec = |label, min, max, init| ExpectedParam { label, min, max, init };
        assert_params_match("panzerdelay", "panzerdelay.dsp", &[
            spec("Dry amp", 0.0, 240.0, 240.0),
            spec("Dry L Pan", 0.0, 240.0, 0.0),
            spec("Dry R Pan", 0.0, 240.0, 240.0),
            spec("Delay unit", 0.0, 3.0, 0.0),
            spec("Filter type", 0.0, 3.0, 0.0),
            spec("Cutoff", 0.0, 240.0, 120.0),
            spec("Resonance", 0.0, 240.0, 120.0),
            spec("L Delay Len", 1.0, 20000.0, 3.0),
            spec("R Delay Len", 1.0, 20000.0, 3.0),
            spec("Tap Amp", 0.0, 240.0, 240.0),
            spec("Wet L Pan", 0.0, 240.0, 240.0),
            spec("Wet R Pan", 0.0, 240.0, 0.0),
            spec("FB Amp", 0.0, 239.0, 150.0),
            spec("FB PingPong", 0.0, 1.0, 0.0),
            spec("FB Spread", 0.0, 240.0, 120.0),
        ]);
    }

    #[test]
    fn automaton_compressor_mkii_set_param_follows_bmx_order() {
        assert_set_param_routes(ACM2.0, ACM2.1, &ACM2_ORDER);
    }

    /// Steady-state output level of a constant 0.5 input after `params` (BMX-scaled 0..65535).
    fn acm2_steady_level(params: &[(u16, i32)]) -> f32 {
        let mut machine = FaustMachine::new(load_dsp(ACM2.0, ACM2.1));
        machine.init(44100);
        params.iter().for_each(|&(p, v)| machine.set_param(p, v));
        (0..400).flat_map(|_| render_left(&mut machine, &[0.5; 256])).last().unwrap()
    }

    #[test]
    fn automaton_compressor_mkii_defaults_pass_through() {
        let level = acm2_steady_level(&[]);
        assert!((level - 0.5).abs() < 1e-4, "ratio 1:1 should be unity, got {level}");
    }

    #[test]
    fn automaton_compressor_mkii_max_ratio_approaches_one_over_25() {
        // Threshold min (-64 dB), ratio 25:1: gain = 1/25 + T/env * 24/25.
        let level = acm2_steady_level(&[(2, 0), (3, 65535)]);
        let t = 10f32.powf(-64.0 / 20.0);
        let expected = 0.5 * (1.0 / 25.0 + t / 0.5 * 24.0 / 25.0);
        assert!((level - expected).abs() < 1e-4, "expected {expected}, got {level}");
    }

    #[test]
    fn automaton_compressor_mkii_gain_is_db_linear() {
        let level = acm2_steady_level(&[(6, 32768)]);
        let expected = 0.5 * 10f32.powf(32.0 / 20.0);
        assert!((level / expected - 1.0).abs() < 1e-3, "expected {expected}, got {level}");
    }

    #[test]
    fn automaton_compressor_mkii_level_mode_is_silent() {
        assert_eq!(acm2_steady_level(&[(0, 65535)]), 0.0);
    }

    #[test]
    fn automaton_compressor_mkii_integer_params_quantize_cleanly() {
        let dsp = load_dsp(ACM2.0, ACM2.1);
        let quantizes = |label: &str, range: std::ops::RangeInclusive<i32>| range.for_each(|v| {
            let got = round_trip_f32(find_param(&dsp.params, label), v as f32);
            assert!((got - v as f32).abs() < 0.5, "{label} {v} round-trips to {got}");
        });
        quantizes("Mode", 0..=2);
        quantizes("Channel", 1..=32);
    }

    #[test]
    fn registry_resolves_automaton_compressor_mkii() {
        assert!(crate::create_faust_machine("Automaton Compressor mkII").is_some());
    // --- Jon Pak EQ-10 ---

    const EQ10_BANDS: [&str; 10] = [
        "31 Hz", "62 Hz", "125 Hz", "250 Hz", "500 Hz",
        "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz",
    ];

    fn eq10_band(label: &'static str) -> ExpectedParam {
        ExpectedParam { label, min: 0.0, max: 65534.0, init: 32767.0 }
    }

    #[test]
    fn eq10_param_ordering_and_ranges() {
        let expected: Vec<_> = EQ10_BANDS.iter().map(|l| eq10_band(l)).collect();
        assert_params_match("eq10", "eq10.dsp", &expected);
    }

    #[test]
    fn eq10_set_param_follows_bmx_order() {
        assert_set_param_routes("eq10", "eq10.dsp", &EQ10_BANDS);
    }

    #[test]
    fn eq10_is_stereo() {
        let dsp = load_dsp("eq10", "eq10.dsp");
        assert_eq!((dsp.num_inputs, dsp.num_outputs), (2, 2));
    }

    #[test]
    fn eq10_registry_entry_exists() {
        assert!(crate::registry::create_faust_machine("EQ-10").is_some());
    }

    fn eq10_rms_gain(params: &[(u16, i32)], freq: f32) -> f32 {
        let mut machine = FaustMachine::new(load_dsp("eq10", "eq10.dsp"));
        machine.init(44100);
        params.iter().for_each(|&(i, v)| machine.set_param(i, v));
        let input = sine(freq, 22050);
        let out: Vec<f32> = input.chunks(256).flat_map(|c| render_left(&mut machine, c)).collect();
        rms(&out[11025..]) / rms(&input[11025..])
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn eq10_defaults_are_flat_below_the_fixed_16k_shelf() {
        assert!((eq10_rms_gain(&[], 1000.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn eq10_16k_control_boosts_8k_by_12_db() {
        let gain = eq10_rms_gain(&[(9, 65535)], 8000.0) / eq10_rms_gain(&[], 8000.0);
        assert!((20.0 * gain.log10() - 12.0).abs() < 0.1, "gain {gain}");
    fn panzerdelay_set_param_follows_bmx_order() {
        assert_set_param_routes("panzerdelay", "panzerdelay.dsp", &PANZER_LABELS);
    }

    #[test]
    fn registry_resolves_panzerdelay() {
        assert!(crate::create_faust_machine("FSM PanzerDelay").is_some());
    }

    /// Normalised 0..65535 value for a raw Buzz value in `min..=max`.
    fn normalised(raw: u32, min: u32, max: u32) -> i32 {
        ((raw - min) as f64 / (max - min) as f64 * 65535.0) as i32
    }

    /// A PanzerDelay with a 100-sample delay on both lines and no feedback.
    fn panzer_100_samples(fb_raw: u32) -> FaustMachine {
        let mut machine = FaustMachine::new(load_dsp("panzerdelay", "panzerdelay.dsp"));
        machine.init(44100);
        machine.set_param(3, normalised(2, 0, 3));
        machine.set_param(7, normalised(100, 1, 20000));
        machine.set_param(8, normalised(100, 1, 20000));
        machine.set_param(12, normalised(fb_raw, 0, 239));
        machine
    }

    /// Render `frames` of stereo silence with a unit impulse at `at`.
    fn render_impulse(machine: &mut FaustMachine, frames: u16, at: usize) -> AudioBuffer {
        let mut buf = AudioBuffer::new(2, frames);
        buf.channel_mut(0)[at] = 1.0;
        buf.channel_mut(1)[at] = 1.0;
        machine.render(&mut buf);
        buf
    }

    #[test]
    fn panzerdelay_echo_arrives_after_exact_delay() {
        let mut machine = panzer_100_samples(0);
        let buf = render_impulse(&mut machine, 6000, 5400);
        let echo = &buf.channel(0)[5400..5520];
        assert!(echo[0] > 0.99, "dry impulse missing: {}", echo[0]);
        assert!(echo[99].abs() < 1e-6 && (echo[100] - 1.0).abs() < 1e-4,
            "echo should land exactly 100 samples later: {} {}", echo[99], echo[100]);
    }

    #[test]
    fn panzerdelay_wet_is_silent_during_first_tick() {
        let mut machine = panzer_100_samples(0);
        let buf = render_impulse(&mut machine, 6000, 1000);
        assert!(buf.channel(0)[1100].abs() < 1e-6, "first tick must not be read back");
    }

    #[test]
    fn panzerdelay_feedback_decays_by_fb_gain() {
        let mut machine = panzer_100_samples(120);
        let buf = render_impulse(&mut machine, 6000, 5400);
        let (first, second) = (buf.channel(0)[5500], buf.channel(0)[5600]);
        assert!((second / first - 0.25).abs() < 1e-3, "fb 120 is 2^-2: {first} {second}");
    }

    // --- Jeskola Reverb 2 behaviour ---

    const RV2_DRY_OUT: u16 = 0;
    const RV2_HIGH_CUTOFF: u16 = 5;

    fn reverb2_with(params: &[(u16, i32)]) -> FaustMachine {
        let mut machine = FaustMachine::new(load_dsp("reverb2", "reverb2.dsp"));
        machine.init(44100);
        params.iter().for_each(|&(p, v)| machine.set_param(p, v));
        machine
    }

    fn sine(freq: f32, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| (i as f32 * freq * core::f32::consts::TAU / 44100.0).sin() * 0.5).collect()
    }

    /// RMS of the wet-only (Dry Out = 0) response to a sine, after the tail builds up.
    fn reverb2_wet_rms(high_cutoff_scaled: i32, freq: f32) -> f32 {
        let mut machine = reverb2_with(&[(RV2_DRY_OUT, 0), (RV2_HIGH_CUTOFF, high_cutoff_scaled)]);
        let out: Vec<f32> = sine(freq, 44100).chunks(256).flat_map(|c| render_left(&mut machine, c)).collect();
        let tail = &out[22050..];
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn reverb2_high_cutoff_attenuates_highs_in_wet_signal() {
        let (open, closed) = (reverb2_wet_rms(65535, 8000.0), reverb2_wet_rms(0, 8000.0));
        assert!(closed < open * 0.1,
            "High Cutoff at 100Hz should cut an 8kHz wet signal: open={open}, closed={closed}");
    }

    // --- Jeskola Freeverb behaviour ---

    const FV_PRE_DELAY: u16 = 2;
    const FV_REV_OUT: u16 = 5;
    const FV_DRY_OUT: u16 = 6;

    /// Set a Buzz byte param (0..127) via the BMX 0..65535 scaling.
    fn set_byte_param(machine: &mut FaustMachine, param: u16, value: i32) {
        machine.set_param(param, (value * 65535 + 63) / 127);
    }

    fn freeverb_with(params: &[(u16, i32)]) -> FaustMachine {
        let mut machine = FaustMachine::new(load_dsp("freeverb", "reverb.dsp"));
        machine.init(44100);
        params.iter().for_each(|&(p, v)| set_byte_param(&mut machine, p, v));
        machine
    }

    /// Render `input` (same signal on both channels) and return the left channel.
    fn render_left(machine: &mut FaustMachine, input: &[f32]) -> Vec<f32> {
        let mut buf = AudioBuffer::new(2, input.len() as u16);
        buf.channel_mut(0).copy_from_slice(input);
        buf.channel_mut(1).copy_from_slice(input);
        machine.render(&mut buf);
        buf.channel(0).to_vec()
    }

    fn render_impulse_left(machine: &mut FaustMachine, frames: usize) -> Vec<f32> {
        let mut input = vec![0.0; frames];
        input[0] = 1.0;
        input.chunks(256).flat_map(|c| render_left(machine, c)).collect()
    }

    fn first_audible(samples: &[f32]) -> Option<usize> {
        samples.iter().position(|s| s.abs() > 1e-6)
    }

    #[test]
    fn freeverb_dry_out_scales_input() {
        // Jeskola: dry = dryOut / 0xFF * scaledry(2); Rev Out 0 => wet ~ -60dB
        let mut machine = freeverb_with(&[(FV_REV_OUT, 0), (FV_DRY_OUT, 64)]);
        let out = (0..40).flat_map(|_| render_left(&mut machine, &[0.5; 256])).last().unwrap();
        let expected = 0.5 * 64.0 / 255.0 * 2.0;
        assert!((out - expected).abs() < 0.01, "expected ~{expected}, got {out}");
    }

    #[test]
    fn freeverb_pre_delay_postpones_tail() {
        // Pre Delay is in ms at the host rate: 50 => 2205 samples at 44.1kHz
        let without = render_impulse_left(&mut freeverb_with(&[(FV_DRY_OUT, 0)]), 8192);
        let with = render_impulse_left(&mut freeverb_with(&[(FV_DRY_OUT, 0), (FV_PRE_DELAY, 50)]), 8192);
        let (onset, delayed) = (first_audible(&without).unwrap(), first_audible(&with).unwrap());
        assert!((delayed as i64 - onset as i64 - 2205).abs() <= 2,
            "pre-delay should shift tail by 2205 samples: {onset} -> {delayed}");
    }

    #[test]
    fn freeverb_silent_with_both_outputs_zero() {
        let mut machine = freeverb_with(&[(FV_REV_OUT, 0), (FV_DRY_OUT, 0)]);
        let peak = render_impulse_left(&mut machine, 8192).iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak < 0.01, "both outputs at 0 should be near-silent, peak={peak}");
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

    // --- Automaton EQ-10 ---

    const EQ10_LABELS: [&str; 10] = [
        "31 Hz", "62 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz",
    ];

    fn eq10_dsp() -> crate::compiler::CompiledDsp {
        load_dsp("automaton_eq10", "automaton_eq10.dsp")
    }

    #[test]
    fn eq10_is_stereo() {
        let dsp = eq10_dsp();
        assert_eq!((dsp.num_inputs, dsp.num_outputs), (2, 2));
    }

    #[test]
    fn eq10_param_ordering_and_ranges() {
        let expected: Vec<ExpectedParam> = EQ10_LABELS.iter()
            .map(|&label| ExpectedParam { label, min: 0.0, max: 65534.0, init: 32767.0 })
            .collect();
        assert_params_match("automaton_eq10", "automaton_eq10.dsp", &expected);
    }

    #[test]
    fn eq10_set_param_follows_bmx_order() {
        assert_set_param_routes("automaton_eq10", "automaton_eq10.dsp", &EQ10_LABELS);
    }

    /// Mirrors the DSP's `word()`: undo the truncating 0..65535 rescale of a 0..65534 word.
    fn eq10_recover_word(slider: f32) -> f32 {
        (slider * 65535.0 / 65534.0).round().min(65534.0)
    }

    #[test]
    fn eq10_word_values_round_trip_exactly() {
        let dsp = eq10_dsp();
        for p in &dsp.params {
            for raw in [0i64, 1, 16384, 32767, 32768, 49151, 65533, 65534] {
                // BMX integer rescale (mb_formats scale_param_value): truncating raw * 65535 / 65534.
                let scaled = (raw * 65535 / 65534) as i32;
                let got = eq10_recover_word(crate::ui_visitor::scaled_to_f32(p, scaled)) as i64;
                assert_eq!(got, raw, "{}: raw {raw} round-trips to {got}", p.label);
    // --- FSM PhatMan ---

    const PHATMAN_ORDER: [&str; 8] = [
        "Dry out", "Reset Phase", "Minimum delay", "Modulation",
        "LFO rate", "Wet out", "Feedback", "Phasing",
    ];

    #[test]
    fn phatman_param_ordering_and_ranges() {
        assert_params_match("phatman", "phatman.dsp", &[
            ExpectedParam { label: "Dry out",       min: 0.0, max: 240.0, init: 240.0 },
            ExpectedParam { label: "Reset Phase",   min: 0.0, max: 240.0, init: 0.0 },
            ExpectedParam { label: "Minimum delay", min: 1.0, max: 100.0, init: 20.0 },
            ExpectedParam { label: "Modulation",    min: 1.0, max: 100.0, init: 10.0 },
            ExpectedParam { label: "LFO rate",      min: 0.0, max: 254.0, init: 10.0 },
            ExpectedParam { label: "Wet out",       min: 0.0, max: 240.0, init: 240.0 },
            ExpectedParam { label: "Feedback",      min: 0.0, max: 128.0, init: 64.0 },
            ExpectedParam { label: "Phasing",       min: 0.0, max: 128.0, init: 32.0 },
        ]);
    }

    #[test]
    fn phatman_set_param_follows_bmx_order() {
        assert_set_param_routes("phatman", "phatman.dsp", &PHATMAN_ORDER);
    }

    #[test]
    fn phatman_integer_values_round_trip_to_nearest_raw() {
        // The DSP recovers raw values with floor(s + 0.5); truncating BMX
        // scaling must never land more than half a step away.
        let dsp = load_dsp("phatman", "phatman.dsp");
        for p in &dsp.params {
            for raw in (p.min as i32)..=(p.max as i32) {
                let got = round_trip_f32(p, raw as f32);
                assert_eq!((got + 0.5).floor() as i32, raw, "{}: raw {raw} -> {got}", p.label);
            }
        }
    }

    fn eq10_sine_rms(params: &[(u16, i32)], freq: f32) -> f32 {
        let mut machine = FaustMachine::new(eq10_dsp());
        machine.init(44100);
        for &(i, v) in params { machine.set_param(i, v); }
        let out = render_left(&mut machine, &sine(freq, 8192));
        (out[4096..].iter().map(|s| s * s).sum::<f32>() / 4096.0).sqrt()
    }

    /// Gain of `params` relative to the default settings, at one frequency.
    fn eq10_gain_db(params: &[(u16, i32)], freq: f32) -> f32 {
        20.0 * (eq10_sine_rms(params, freq) / eq10_sine_rms(&[], freq)).log10()
    }

    /// Absolute gain at the defaults, against the 1 kHz reference.
    fn eq10_default_db(freq: f32) -> f32 {
        20.0 * (eq10_sine_rms(&[], freq) / eq10_sine_rms(&[], 1.0e3)).log10()
    }

    #[test]
    fn eq10_defaults_are_flat_in_midrange() {
        assert!(eq10_default_db(250.0).abs() < 0.05);
    }

    #[test]
    fn eq10_fixed_high_shelf_is_minus_6db_at_16k() {
        assert!((eq10_default_db(16000.0) + 6.0).abs() < 0.3);
    }

    #[test]
    fn eq10_bands_sit_one_slot_below_their_labels() {
        // "16 kHz" (index 9) at max boosts 8 kHz by 12 dB.
        let db = eq10_gain_db(&[(9, 65535)], 8000.0);
        assert!((db - 12.0).abs() < 0.3, "8 kHz gain {db}");
    }

    #[test]
    fn eq10_31hz_slider_is_dead() {
        assert!(eq10_gain_db(&[(0, 0)], 100.0).abs() < 0.01);
        assert!(eq10_gain_db(&[(1, 0)], 31.25) < -5.0, "62 Hz slider drives the 31 Hz shelf");
    }

    #[test]
    fn registry_resolves_automaton_eq10() {
        assert!(crate::create_faust_machine("Automaton EQ-10").is_some());
    fn phatman_with(params: &[(u16, f32)]) -> FaustMachine {
        let dsp = load_dsp("phatman", "phatman.dsp");
        let scaled: Vec<(u16, i32)> = params.iter().map(|&(i, raw)| {
            let p = &dsp.params[i as usize];
            (i, ((raw - p.min) / (p.max - p.min) * 65535.0) as i32)
        }).collect();
        let mut machine = FaustMachine::new(dsp);
        machine.init(44100);
        scaled.iter().for_each(|&(i, v)| machine.set_param(i, v));
        machine
    }

    #[test]
    fn phatman_wet_off_is_identity() {
        let mut machine = phatman_with(&[(5, 0.0)]);
        let input = sine(440.0, 4096);
        let out: Vec<f32> = input.chunks(256).flat_map(|c| render_left(&mut machine, c)).collect();
        let err = input.iter().zip(&out).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(err < 1e-6, "dry 0 dB / wet off should pass input through, max err {err}");
    }

    #[test]
    fn phatman_wet_only_is_delayed_copy() {
        // Dry off, LFO frozen-ish (rate 0): the wet tap settles near
        // Min + Depth/2 = 88.2 + 22 samples after the startup glide.
        let mut machine = phatman_with(&[(0, 0.0)]);
        let frames = 44100;
        let mut input = vec![0.0; frames];
        input[30000] = 1.0;
        let out: Vec<f32> = input.chunks(256).flat_map(|c| render_left(&mut machine, c)).collect();
        let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert!((30088..30133).contains(&peak), "wet peak at {peak}");
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


    // --- Chimp's PitchShifter ---

    const CPS_SHIFT: u16 = 0;
    const CPS_DIRECTION: u16 = 1;
    const CPS_DRY: u16 = 2;
    const CPS_WET: u16 = 3;

    #[test]
    fn chimp_pitchshifter_param_ordering_and_ranges() {
        assert_params_match("chimp_pitchshifter", "chimp_pitchshifter.dsp", &[
            ExpectedParam { label: "Shift",     min: 0.0, max: 2400.0, init: 1200.0 },
            ExpectedParam { label: "Direction", min: 0.0, max: 1.0,    init: 0.0 },
            ExpectedParam { label: "Dry out",   min: 0.0, max: 128.0,  init: 128.0 },
            ExpectedParam { label: "Wet out",   min: 0.0, max: 128.0,  init: 128.0 },
        ]);
    }

    #[test]
    fn chimp_pitchshifter_set_param_follows_bmx_order() {
        assert_set_param_routes("chimp_pitchshifter", "chimp_pitchshifter.dsp",
            &["Shift", "Direction", "Dry out", "Wet out"]);
    }

    /// Scale a raw Buzz value (0..max) into the BMX 0..65535 range.
    fn scale_raw(raw: i32, max: i32) -> i32 { raw * 65535 / max }

    fn chimp_with(raw: &[(u16, i32, i32)]) -> FaustMachine {
        let mut machine = FaustMachine::new(load_dsp("chimp_pitchshifter", "chimp_pitchshifter.dsp"));
        machine.init(44100);
        raw.iter().for_each(|&(p, v, max)| machine.set_param(p, scale_raw(v, max)));
        machine
    }

    /// 256-block render of a ramp signal that is never silent.
    fn chimp_ramp(machine: &mut FaustMachine, frames: usize) -> Vec<f32> {
        let input: Vec<f32> = (0..frames).map(|i| (i % 1000) as f32 / 1000.0 + 0.1).collect();
        input.chunks(256).flat_map(|c| render_left(machine, c)).collect()
    }

    #[test]
    fn chimp_pitchshifter_octave_down_repeats_samples() {
        // Shift 1200 down, dry 64 (0%), wet 128 (100%): every input sample appears twice
        let mut m = chimp_with(&[(CPS_SHIFT, 1200, 2400), (CPS_DRY, 64, 128)]);
        let input: Vec<f32> = (1..=600).map(|i| i as f32 / 1000.0).collect();
        let out: Vec<f32> = input.chunks(256).flat_map(|c| render_left(&mut m, c)).collect();
        assert!((out[10] - out[11]).abs() < 1e-6 || (out[9] - out[10]).abs() < 1e-6);
        assert!((out[400] - input[200]).abs() < 1e-5 || (out[400] - input[201]).abs() < 1e-5);
    }

    #[test]
    fn chimp_pitchshifter_octave_up_skips_samples() {
        let mut m = chimp_with(&[(CPS_SHIFT, 1200, 2400), (CPS_DIRECTION, 1, 1), (CPS_DRY, 64, 128)]);
        let out = chimp_ramp(&mut m, 2048);
        // after the 1024-sample splice, the head advances two input samples per output sample
        let step = out[1500] - out[1499];
        assert!((step.abs() - 0.002).abs() < 1e-4, "got step {step}");
    }

    #[test]
    fn chimp_pitchshifter_zero_shift_passes_input() {
        let mut m = chimp_with(&[(CPS_SHIFT, 0, 2400), (CPS_DRY, 100, 128)]);
        let out = chimp_ramp(&mut m, 512);
        assert!((out[300] - ((300 % 1000) as f32 / 1000.0 + 0.1)).abs() < 1e-6);
    }

    #[test]
    fn chimp_pitchshifter_dry_plus_wet_128_is_silent() {
        let mut m = chimp_with(&[(CPS_DRY, 96, 128), (CPS_WET, 32, 128)]);
        let peak = chimp_ramp(&mut m, 1024).iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert_eq!(peak, 0.0);
    }

    #[test]
    fn chimp_pitchshifter_cuts_silent_blocks() {
        let mut m = chimp_with(&[(CPS_DRY, 128, 128)]);
        let mut input = vec![0.5f32; 256];
        input.extend(vec![0.0f32; 512]);
        let out: Vec<f32> = input.chunks(256).flat_map(|c| render_left(&mut m, c)).collect();
        assert!(out[256..].iter().all(|s| *s == 0.0));
        assert!(out[..256].iter().any(|s| s.abs() > 0.1));
    }

    #[test]
    fn registry_resolves_chimp_pitchshifter() {
        assert!(crate::create_faust_machine("Chimp's PitchShifter").is_some());
    }

    #[test]
    fn registry_resolves_degrader() {
        assert!(crate::create_faust_machine("Arguelles Degrader").is_some());
    }

    fn render_degrader(params: &[(u16, i32)], input: &[f32]) -> Vec<f32> {
        let source = std::fs::read_to_string(
            format!("{}/../../faust/degrader.dsp", env!("CARGO_MANIFEST_DIR"))
        ).unwrap();
        let mut machine = FaustMachine::from_source("degrader", &source).unwrap();
        machine.init(44100);
        params.iter().for_each(|&(i, v)| machine.set_param(i, v));
        let mut buf = AudioBuffer::new(2, input.len() as u16);
        buf.channel_mut(0).copy_from_slice(input);
        buf.channel_mut(1).copy_from_slice(input);
        machine.render(&mut buf);
        buf.channel(0).to_vec()
    }

    #[test]
    fn degrader_holds_for_rate_plus_one_samples() {
        // Smp. Rate 5 (65535 * 4 / 31): captures at n = 4, 10, 16, ...
        let input: Vec<f32> = (0..32).map(|n| (n + 1) as f32 / 64.0).collect();
        let out = render_degrader(&[(0, 8456)], &input);
        assert_eq!(&out[..4], &[0.0; 4]);
        assert!(out[4..10].iter().all(|&y| y == input[4]));
        assert!(out[10..16].iter().all(|&y| y == input[10]));
    }

    #[test]
    fn degrader_truncates_toward_zero() {
        // Bit Depth 6 (65535 * 4 / 14): grid of 1024 / 32768. Smp. Rate 1
        // holds each capture for two samples (captures at n = 0, 2).
        let (a, b) = (-3194.0 / 32768.0, 3194.0 / 32768.0);
        let out = render_degrader(&[(1, 18724)], &[a, a, b, b]);
        let (qa, qb) = (-3072.0 / 32768.0, 3072.0 / 32768.0);
        assert_eq!(out, vec![qa, qa, qb, qb]);
    fn registry_resolves_automaton_eq7() {
        assert!(crate::create_faust_machine("Automaton EQ-7").is_some());
    fn registry_resolves_automaton_parametric_eq() {
        assert!(crate::create_faust_machine("Automaton Parametric EQ").is_some());
    }

    // --- Automaton Parametric EQ ---

    const APEQ: (&str, &str) = ("automaton_parametric_eq", "automaton_parametric_eq.dsp");
    const APEQ_LABELS: [&str; 9] = [
        "LPF-Gain", "LPF-Freq.", "LPF-Shelf", "HPF-Gain", "HPF-Freq.", "HPF-Shelf", "Gain", "Freq.", "BWidth",
    ];
    const APEQ_INITS: [f32; 9] = [32767.0, 32767.0, 0.0, 32767.0, 32767.0, 0.0, 32767.0, 32767.0, 32767.0];
    const APEQ_BAND_GAIN: u16 = 6;

    #[test]
    fn apeq_compiles_stereo() {
        let dsp = load_dsp(APEQ.0, APEQ.1);
        assert_eq!((dsp.num_inputs, dsp.num_outputs), (2, 2));
    }

    #[test]
    fn apeq_param_ordering_and_ranges() {
        let expected: Vec<ExpectedParam> = APEQ_LABELS.iter().zip(APEQ_INITS)
            .map(|(&label, init)| ExpectedParam { label, min: 0.0, max: 65534.0, init })
            .collect();
        assert_params_match(APEQ.0, APEQ.1, &expected);
    }

    #[test]
    fn apeq_set_param_follows_bmx_order() {
        assert_set_param_routes(APEQ.0, APEQ.1, &APEQ_LABELS);
    }

    /// Scale a Buzz word value (0..65534) the way the BMX loader does.
    fn apeq_with(params: &[(u16, i32)]) -> FaustMachine {
        let mut machine = FaustMachine::new(load_dsp(APEQ.0, APEQ.1));
        machine.init(44100);
        params.iter().for_each(|&(p, v)| machine.set_param(p, (v as i64 * 65535 / 65534) as i32));
        machine
    }

    fn render_blocks(machine: &mut FaustMachine, input: &[f32]) -> Vec<f32> {
        input.chunks(256).flat_map(|c| render_left(machine, c)).collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn apeq_defaults_are_identity() {
        let input = sine(1000.0, 4096);
        let out = render_blocks(&mut apeq_with(&[]), &input);
        let worst = out.iter().zip(&input).map(|(a, b)| (a - b).abs()).enumerate()
            .fold((0, 0.0f32), |m, (i, d)| if d > m.1 { (i, d) } else { m });
        assert!(worst.1 < 1e-5, "defaults should pass input through, worst {worst:?}");
    }

    #[test]
    fn apeq_band_gain_boosts_centre_by_36_db() {
        // Freq. 32767 = 1052.9 Hz; Gain 65534 = +36 dB
        let out = render_blocks(&mut apeq_with(&[(APEQ_BAND_GAIN, 65534)]), &sine(1052.9, 16384));
        let gain_db = 20.0 * (peak(&out[8192..]) / 0.5).log10();
        assert!((gain_db - 36.0).abs() < 0.3, "expected +36 dB, got {gain_db}");
    }

    #[test]
    fn apeq_cuts_output_on_silent_blocks() {
        let mut input = vec![0.0; 2048];
        input[300] = 1.0;
        let out = render_blocks(&mut apeq_with(&[(0, 65534)]), &input);
        assert!(peak(&out[256..512]) > 0.1, "impulse block should pass");
        assert_eq!(peak(&out[512..]), 0.0, "shelf tail is skipped on silent blocks: {:?}", &out[510..520]);
    }

    #[test]
    fn registry_resolves_phatman() {
        assert!(crate::create_faust_machine("FSM PhatMan").is_some());
    // --- FSM SprayMan ---

    const SPRAYMAN_LABELS: [&str; 13] = [
        "Dry out", "Feedback", "Fullness", "Richness", "Density", "Scattering", "Fatness",
        "Attack", "Sustain", "Release", "Wet out", "Pan", "Spread",
    ];

    #[test]
    fn sprayman_param_ordering_and_ranges() {
        let ranges = [(0.0, 240.0, 240.0), (0.0, 99.0, 0.0), (0.0, 100.0, 0.0), (4.0, 24.0, 12.0),
            (0.0, 64.0, 48.0), (0.0, 64.0, 48.0), (0.0, 64.0, 16.0), (0.0, 240.0, 60.0),
            (0.0, 240.0, 120.0), (0.0, 240.0, 32.0), (0.0, 240.0, 240.0), (0.0, 240.0, 120.0),
            (0.0, 100.0, 50.0)];
        let expected: Vec<ExpectedParam> = SPRAYMAN_LABELS.iter().zip(ranges)
            .map(|(&label, (min, max, init))| ExpectedParam { label, min, max, init })
            .collect();
        assert_params_match("sprayman", "sprayman.dsp", &expected);
    }

    #[test]
    fn sprayman_set_param_follows_bmx_order() {
        assert_set_param_routes("sprayman", "sprayman.dsp", &SPRAYMAN_LABELS);
    }

    fn sprayman_with(params: &[(u16, i32)]) -> FaustMachine {
        let mut machine = FaustMachine::new(load_dsp("sprayman", "sprayman.dsp"));
        machine.init(44100);
        params.iter().for_each(|&(p, v)| machine.set_param(p, v));
        machine
    }

    fn render_blocks(machine: &mut FaustMachine, input: &[f32], blocks: usize) -> Vec<f32> {
        (0..blocks).flat_map(|_| render_left(machine, input)).collect()
    }

    #[test]
    fn sprayman_wet_off_passes_dry() {
        // Wet out = 0 (raw 0 -> BMX 0): output is exactly the dry input at 0 dB.
        let mut machine = sprayman_with(&[(10, 0)]);
        let out = render_blocks(&mut machine, &[0.25; 256], 4);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn sprayman_wet_grains_start_after_initial_wait() {
        // Dry out = 0: silence until the first grain (~5800 samples or later), then output.
        let mut machine = sprayman_with(&[(0, 0)]);
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.3).sin() * 0.5).collect();
        let out = render_blocks(&mut machine, &input, 160);
        let first = first_audible(&out).expect("grains should sound");
        assert!(first >= 5800, "first grain at {first}");
    }

    #[test]
    fn registry_resolves_sprayman() {
        assert!(crate::create_faust_machine("FSM SprayMan").is_some());
    }

    #[test]
    fn registry_returns_none_for_unknown() {
        assert!(crate::create_faust_machine("Unknown Machine XYZ").is_none());
    }

    // --- Automaton EQ-7 ---

    const EQ7_BANDS: [&str; 7] = ["63 Hz", "160 Hz", "400 Hz", "1 kHz", "2.5 kHz", "6.3 kHz", "16 kHz"];

    fn eq7_band(label: &'static str) -> ExpectedParam {
        ExpectedParam { label, min: 0.0, max: 65534.0, init: 32767.0 }
    }

    #[test]
    fn eq7_param_ordering_and_ranges() {
        let expected: Vec<_> = EQ7_BANDS.iter().map(|l| eq7_band(l)).collect();
        assert_params_match("automaton_eq7", "automaton_eq7.dsp", &expected);
    }

    #[test]
    fn eq7_set_param_follows_bmx_order() {
        assert_set_param_routes("automaton_eq7", "automaton_eq7.dsp", &EQ7_BANDS);
    }

    fn eq7_with(params: &[(u16, i32)]) -> FaustMachine {
        let mut machine = FaustMachine::new(load_dsp("automaton_eq7", "automaton_eq7.dsp"));
        machine.init(44100);
        params.iter().for_each(|&(p, v)| machine.set_param(p, v));
        machine
    }

    /// Render `input` through `machine` in 256-frame blocks (the Buzz work size).
    fn render_blocks_left(machine: &mut FaustMachine, input: &[f32]) -> Vec<f32> {
        input.chunks(256).flat_map(|c| render_left(machine, c)).collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// Steady-state gain (dB) of a sine at `freq` with the given BMX-scaled params.
    fn eq7_gain_db(params: &[(u16, i32)], freq: f32) -> f32 {
        let input = sine(freq, 32768);
        let out = render_blocks_left(&mut eq7_with(params), &input);
        20.0 * (rms(&out[16384..]) / rms(&input[16384..])).log10()
    }

    /// BMX-scaled value (0..65535) for a raw Buzz word in 0..65534.
    fn eq7_scaled(raw: i32) -> i32 {
        (raw as i64 * 65535 / 65534) as i32
    }

    #[test]
    fn eq7_defaults_are_identity() {
        let input = sine(1000.0, 1024);
        let out = render_blocks_left(&mut eq7_with(&[]), &input);
        let err = out.iter().zip(&input).map(|(o, i)| (o - i).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-6, "max deviation from identity {err}");
    }

    #[test]
    fn eq7_band_max_boosts_36db_at_centre() {
        // 160 Hz band is centred at 125 Hz (measured), the others at their label.
        let centres = [(2u16, 400.0), (3, 1000.0), (4, 2500.0), (1, 125.0)];
        for (band, freq) in centres {
            let g = eq7_gain_db(&[(band, eq7_scaled(65534))], freq);
            assert!((g - 36.0).abs() < 0.3, "band {band} at {freq} Hz: {g} dB");
        }
    }

    #[test]
    fn eq7_quarter_word_cuts_18db() {
        let g = eq7_gain_db(&[(3, eq7_scaled(16384))], 1000.0);
        assert!((g + 18.0).abs() < 0.2, "1 kHz at 16384: {g} dB");
    }

    #[test]
    fn eq7_shelves_reach_half_gain_at_corner() {
        let low = eq7_gain_db(&[(0, eq7_scaled(65534))], 63.0);
        let high = eq7_gain_db(&[(6, eq7_scaled(0))], 16000.0);
        assert!((low - 18.0).abs() < 0.3, "63 Hz shelf at 63 Hz: {low} dB");
        assert!((high + 18.0).abs() < 0.3, "16 kHz shelf at 16 kHz: {high} dB");
    }

    #[test]
    fn eq7_silent_block_cuts_tail() {
        // work_returns_false: a block of all-zero input yields silence even
        // while the 1 kHz peaking filter is still ringing.
        let mut machine = eq7_with(&[(3, eq7_scaled(65534))]);
        let mut input = vec![0.0; 1024];
        input[200] = 1.0;
        let out = render_blocks_left(&mut machine, &input);
        assert!(out[255].abs() > 1e-4, "tail should ring inside the impulse block");
        assert!(out[256..].iter().all(|s| *s == 0.0), "silent blocks must be cut");
    #[test]
    fn registry_resolves_smootherdrive() {
        assert!(crate::create_faust_machine("Dave's Smootherdrive").is_some());
    }

    // --- Dave's SmootherDrive ---

    const SD_THRESHOLD: u16 = 0;
    const SD_INTENSITY: u16 = 1;

    #[test]
    fn smootherdrive_param_ordering_and_ranges() {
        assert_params_match("smootherdrive", "smootherdrive.dsp", &[
            ExpectedParam { label: "Threshold", min: 0.0, max: 32767.0, init: 32767.0 },
            ExpectedParam { label: "Intensity", min: 0.0, max: 100.0,   init: 0.0 },
        ]);
    }

    #[test]
    fn smootherdrive_set_param_follows_bmx_order() {
        assert_set_param_routes("smootherdrive", "smootherdrive.dsp", &["Threshold", "Intensity"]);
    }

    /// Set a raw Buzz value via the BMX truncating 0..65535 scaling.
    fn set_raw_param(machine: &mut FaustMachine, param: u16, raw: i32, max: i32) {
        machine.set_param(param, (raw as i64 * 65535 / max as i64) as i32);
    }

    fn smootherdrive_out(threshold: i32, intensity: i32, input: f32) -> f32 {
        let mut machine = FaustMachine::new(load_dsp("smootherdrive", "smootherdrive.dsp"));
        machine.init(44100);
        set_raw_param(&mut machine, SD_THRESHOLD, threshold, 32767);
        set_raw_param(&mut machine, SD_INTENSITY, intensity, 100);
        *render_left(&mut machine, &[input; 64]).last().unwrap()
    }

    fn assert_close(got: f32, expected: f32, tol: f32) {
        assert!((got - expected).abs() < tol, "expected ~{expected}, got {got}");
    }

    #[test]
    fn smootherdrive_intensity_zero_is_bypass() {
        assert_close(smootherdrive_out(4096, 0, 0.3), 0.3, 1e-7);
    }

    #[test]
    fn smootherdrive_linear_gain_below_threshold() {
        // Measured: T=4096, I=25 gives a small-signal gain of exactly 19/3.
        assert_close(smootherdrive_out(4096, 25, -0.05), -0.05 * 19.0 / 3.0, 1e-5);
    }

    #[test]
    fn smootherdrive_zero_threshold_is_quarter_sine() {
        let x = 16000.0 / 32768.0;
        assert_close(smootherdrive_out(0, 50, x), (std::f32::consts::FRAC_PI_2 * x).sin(), 1e-5);
    }

    #[test]
    fn smootherdrive_full_scale_maps_to_full_scale() {
        assert_close(smootherdrive_out(12288, 63, 1.0), 1.0, 1e-5);
    }

    // --- Machine-code disk cache tests ---

    #[test]
    fn cache_miss_returns_none() {
        assert!(crate::cache::try_load("no-such-machine", "process = _;").is_none());
    }

    #[test]
    fn cache_round_trip_saves_and_loads() {
        let name = "cache_round_trip";
        let source = "process = *(0.5), *(0.5); // cache round-trip marker";
        let dsp = crate::compiler::compile_cached(name, source, None).unwrap();
        assert_eq!(dsp.num_inputs, 2);
        let factory = crate::cache::try_load(name, source)
            .expect("factory should be cached after compile_cached");
        unsafe { crate::ffi::deleteCDSPFactory(factory); }
    }

    // --- Argüelles KFlanger ---

    #[test]
    fn kflanger_param_ordering_and_ranges() {
        assert_params_match("kflanger", "kflanger.dsp", &[
            ExpectedParam { label: "Min Flanger",  min: 0.0, max: 254.0,   init: 0.0 },
            ExpectedParam { label: "Max Flanger",  min: 0.0, max: 254.0,   init: 127.0 },
            ExpectedParam { label: "Flanger Mode", min: 0.0, max: 1.0,     init: 0.0 },
            ExpectedParam { label: "Sweep Speed",  min: 0.0, max: 254.0,   init: 1.0 },
            ExpectedParam { label: "Feedback",     min: 0.0, max: 65534.0, init: 32767.0 },
    // --- Automaton Compressor ---

    const AC_THRESHOLD: u16 = 0;
    const AC_RATIO: u16 = 1;

    #[test]
    fn automaton_compressor_param_ordering_and_ranges() {
        assert_params_match("automaton_compressor", "automaton_compressor.dsp", &[
            ExpectedParam { label: "Threshold", min: 0.0, max: 65534.0, init: 47512.0 },
            ExpectedParam { label: "Ratio",     min: 0.0, max: 65534.0, init: 0.0 },
            ExpectedParam { label: "Attack",    min: 0.0, max: 65534.0, init: 12577.0 },
            ExpectedParam { label: "Release",   min: 0.0, max: 65534.0, init: 7452.0 },
            ExpectedParam { label: "Gain",      min: 0.0, max: 65534.0, init: 0.0 },
    // --- Chun-Yu's TouchWah ---

    const TOUCHWAH_PARAMS: [&str; 6] = [
        "Resonance", "Min Cutoff (Hz)", "Max Cutoff (Hz)", "Changes/Sec", "In->SinCurve?", "ScaleFactor",
    ];

    #[test]
    fn touchwah_param_ordering_and_ranges() {
        assert_params_match("touchwah", "chun-yu-s-touchwah.dsp", &[
            ExpectedParam { label: "Resonance",       min: 10.0, max: 141.0,   init: 129.0 },
            ExpectedParam { label: "Min Cutoff (Hz)", min: 1.0,  max: 22049.0, init: 256.0 },
            ExpectedParam { label: "Max Cutoff (Hz)", min: 2.0,  max: 22050.0, init: 11025.0 },
            ExpectedParam { label: "Changes/Sec",     min: 1.0,  max: 128.0,   init: 32.0 },
            ExpectedParam { label: "In->SinCurve?",   min: 0.0,  max: 1.0,     init: 0.0 },
            ExpectedParam { label: "ScaleFactor",     min: 1.0,  max: 16.0,    init: 1.0 },
    #[test]
    fn fsm_chorus_set_param_follows_bmx_order() {
        assert_set_param_routes("fsm_chorus", "fsm_chorus.dsp", &[
            "Dry out", "Minimum delay", "Modulation", "LFO rate", "Wet out", "Feedback",
        ]);
    }

    #[test]
    fn kflanger_set_param_follows_bmx_order() {
        assert_set_param_routes("kflanger", "kflanger.dsp", &[
            "Min Flanger", "Max Flanger", "Flanger Mode", "Sweep Speed", "Feedback",
        ]);
    }

    const KF_MODE: u16 = 2;
    const KF_SPEED: u16 = 3;

    fn kflanger_with(params: &[(u16, i32)]) -> FaustMachine {
        let mut machine = FaustMachine::new(load_dsp("kflanger", "kflanger.dsp"));
        machine.init(44100);
        params.iter().for_each(|&(p, v)| machine.set_param(p, v));
        machine
    }

    /// Renders `blocks` successive 256-sample blocks of the same input.
    fn kflanger_blocks(machine: &mut FaustMachine, input: &[f32], blocks: usize) -> Vec<Vec<f32>> {
        (0..blocks).map(|_| render_left(machine, input)).collect()
    }

    /// Ramp input whose value encodes its position, so delays are readable.
    fn kflanger_ramp() -> Vec<f32> {
        (0..256).map(|i| (i as f32 + 1.0) / 512.0).collect()
    }

    #[test]
    fn kflanger_defaults_add_undelayed_wet() {
        let input = kflanger_ramp();
        let out = render_left(&mut kflanger_with(&[]), &input);
        out.iter().zip(&input).for_each(|(y, x)| assert!((y - 2.0 * x).abs() < 1e-3));
    }

    #[test]
    fn kflanger_mode_1_cancels_at_zero_delay() {
        let out = render_left(&mut kflanger_with(&[(KF_MODE, 65535)]), &kflanger_ramp());
        assert!(out.iter().all(|y| y.abs() < 1e-4), "dry - wet should cancel at delay 0");
    }

    #[test]
    fn kflanger_full_speed_adds_one_sample_of_delay_per_block() {
        let input = kflanger_ramp();
        let blocks = kflanger_blocks(&mut kflanger_with(&[(KF_SPEED, 65535)]), &input, 4);
        // Block k reads the wet signal k samples late (speed 254/256 per block).
        blocks.iter().enumerate().skip(1).for_each(|(k, out)| {
            let wet = out[100] - input[100];
            assert!((wet - input[100 - k]).abs() < 1e-3, "block {k}: wet {wet}");
        });
    }

    #[test]
    fn kflanger_silent_block_outputs_zero_despite_feedback() {
        // Feedback 65534 rings for seconds; Work returns false on silent input.
        let mut machine = kflanger_with(&[(4, 65535)]);
        let mut impulse = vec![0.0; 256];
        impulse[10] = 0.5;
        let first = render_left(&mut machine, &impulse);
        let silent = render_left(&mut machine, &[0.0; 256]);
        assert!(first[20].abs() > 0.1, "feedback should ring inside the input block");
        assert!(silent.iter().all(|y| *y == 0.0), "silent block must be cut to zero");
    fn automaton_compressor_set_param_follows_bmx_order() {
        assert_set_param_routes("automaton_compressor", "automaton_compressor.dsp", &[
            "Threshold", "Ratio", "Attack", "Release", "Gain",
        ]);
    }

    /// Steady-state output/input peak ratio for a 1 kHz, 0.5-amplitude sine.
    fn automaton_compressor_sine_gain(params: &[(u16, i32)]) -> f32 {
        let mut machine = FaustMachine::new(load_dsp("automaton_compressor", "automaton_compressor.dsp"));
        machine.init(44100);
        params.iter().for_each(|&(p, v)| machine.set_param(p, v));
        let out: Vec<f32> = sine(1000.0, 44100).chunks(256).flat_map(|c| render_left(&mut machine, c)).collect();
        out[33075..].iter().fold(0.0f32, |m, s| m.max(s.abs())) / 0.5
    }

    #[test]
    fn automaton_compressor_defaults_are_unity() {
        let g = automaton_compressor_sine_gain(&[]);
        assert!((g - 1.0).abs() < 1e-3, "1:1 ratio should pass through, gain={g}");
    }

    #[test]
    fn automaton_compressor_gain_reduction_saturates_at_one_over_ratio() {
        // Threshold -64 dB, ratio 25:1: linear-amplitude law gives ~1/25 for loud input.
        let g = automaton_compressor_sine_gain(&[(AC_THRESHOLD, 0), (AC_RATIO, 65535)]);
        assert!(g > 0.035 && g < 0.05, "expected gain near 1/25, got {g}");
    fn touchwah_set_param_follows_bmx_order() {
        assert_set_param_routes("touchwah", "chun-yu-s-touchwah.dsp", &TOUCHWAH_PARAMS);
    }

    /// Render `frames` samples of a mono-in-both-channels signal through TouchWah and return channel 0.
    fn touchwah_render(frames: u16, signal: impl Fn(usize) -> f32, params: &[(u16, u16)]) -> Vec<f32> {
        let mut machine = FaustMachine::new(load_dsp("touchwah", "chun-yu-s-touchwah.dsp"));
        machine.init(44100);
        params.iter().for_each(|&(i, v)| machine.set_param(i, i32::from(v)));
        let mut buf = AudioBuffer::new(2, frames);
        (0..frames as usize).for_each(|n| {
            buf.channel_mut(0)[n] = signal(n);
            buf.channel_mut(1)[n] = signal(n);
        });
        machine.render(&mut buf);
        buf.channel(0)[..frames as usize].to_vec()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn sine_1k(n: usize) -> f32 {
        0.1 * (2.0 * std::f32::consts::PI * 1000.0 * n as f32 / 44100.0).sin()
    }

    #[test]
    fn touchwah_silent_input_gives_silence() {
        let out = touchwah_render(1024, |_| 0.0, &[]);
        assert!(out.iter().all(|&s| s == 0.0), "silent input must give silent output");
    }

    #[test]
    fn touchwah_scale_factor_divides_output() {
        let plain = touchwah_render(2048, sine_1k, &[]);
        let scaled = touchwah_render(2048, sine_1k, &[(5, 65535)]); // ScaleFactor = 16
        let ratio = rms(&scaled[1024..]) / rms(&plain[1024..]);
        assert!((ratio - 1.0 / 16.0).abs() < 0.002, "ScaleFactor 16 should divide by 16, ratio={ratio}");
    }

    #[test]
    fn touchwah_cutoff_follows_input_level() {
        // Min cutoff 1 kHz-ish, max 22 kHz: a loud 10 kHz tone opens the filter, a quiet one stays shut.
        let tone = |amp: f32| move |n: usize| amp * (2.0 * std::f32::consts::PI * 10000.0 * n as f32 / 44100.0).sin();
        let params = [(1, 1000), (3, 65535)]; // Min Cutoff low, Changes/Sec = 128
        let quiet = touchwah_render(8192, tone(0.01), &params);
        let loud = touchwah_render(8192, tone(0.8), &params);
        let (gq, gl) = (rms(&quiet[4096..]) / 0.01, rms(&loud[4096..]) / 0.8);
        assert!(gl > 4.0 * gq, "loud input should open the low-pass: gain quiet={gq}, loud={gl}");
    fn fsm_chorus_is_registered() {
        assert!(crate::registry::create_faust_machine("FSM Chorus").is_some());
    }
}
