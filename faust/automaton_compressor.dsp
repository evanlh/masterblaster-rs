// Automaton Compressor (Jon Pak) — clean-room Faust port.
//
// Derived only from the machine's parameter metadata/display text and the
// buzz-machines-oracles measurements (no machine source read):
//
// * Parameter units, fitted from describe.json display text (raw v in 0..65534):
//     Threshold dB = -64 + 80 v/65534        (0 dBFS = full scale)
//     Ratio        = 25^(v/65534)            (1:1 .. 25:1, exponential)
//     Attack ms    = 0.1 + 199.9 v/65534
//     Release ms   = 50 + 2950 v/65534
//     Gain dB      = 64 v/65534
// * Detector: per-sample peak follower on |x|, one-pole step toward |x| with
//   coefficient 5/(time_ms * sr/1000), attack coefficient when |x| > env,
//   release otherwise; starts at 0. The factor 5 was fitted on stepped-sine
//   (attack/release sweeps); with it every development render (all probes)
//   matches to ~1e-6 relative RMS. Attack at its minimum gives a coefficient
//   > 1 (overshoot), which the oracle shows too, so it is not clamped.
// * Gain law (linear amplitude, not dB): above threshold T the output
//   amplitude is T + (env - T)/ratio, i.e. gain = (T + (env - T)/ratio)/env;
//   gain = 1 below T. Hence the gain reduction saturates at 1/ratio.
//   Evidence: steady-state sine gains give a ratio/threshold-independent
//   implied envelope, and loud steps converge to exactly -20log10(ratio).
// * The machine is mono; this port is 2-in/2-out with the detector fed by
//   (L+R)/2 and the gain applied to both channels.
// * Known accepted difference: work_returns_false (the original stops output
//   when the input goes silent); masterblaster's Faust path keeps running.
import("stdfaust.lib");

threshold = rint(hslider("[0]Threshold", 47512, 0, 65534, 1));
ratio_raw = rint(hslider("[1]Ratio", 0, 0, 65534, 1));
attack    = rint(hslider("[2]Attack", 12577, 0, 65534, 1));
release   = rint(hslider("[3]Release", 7452, 0, 65534, 1));
gain_raw  = rint(hslider("[4]Gain", 0, 0, 65534, 1));

span(v) = v / 65534.0;
thresholdLin = ba.db2linear(-64.0 + 80.0 * span(threshold));
ratio = pow(25.0, span(ratio_raw));
attackMs = 0.1 + 199.9 * span(attack);
releaseMs = 50.0 + 2950.0 * span(release);
makeup = ba.db2linear(64.0 * span(gain_raw));

coef(ms) = 5.0 / (ms * ma.SR / 1000.0);

follower(v) = step ~ _
with {
    step(e) = e + select2(v > e, coef(releaseMs), coef(attackMs)) * (v - e);
};

compGain(env) = select2(env > thresholdLin, 1.0,
    (thresholdLin + (env - thresholdLin) / ratio) / max(env, 1e-30));

stereoGain(l, r) = ((l + r) * 0.5) : abs : follower : compGain : *(makeup);

process(l, r) = l * g, r * g with { g = stereoGain(l, r); };
