// Automaton Compressor mkII (Jon Pak) — clean-room Faust port.
//
// Derived only from info.json / describe.json / triage.json and the
// buzz-machines-oracles measurements (no machine source was read).
//
// Parameter mappings (match every describe.json display string to < 0.0005):
//   Threshold dB = -64 + 80 * v/65534          (attributes: min -64, max +16 dB)
//   1/Ratio      = 1 - (v/65534) * (1 - 1/25)  (1:1 .. 25:1, linear in slope)
//   Attack ms    = 0.1 + 199.9 * v/65534
//   Release ms   = 50 + 2950 * v/65534
//   Gain dB      = 64 * v/65534
//
// Signal path (recovered from sample-level gain = output/input on the
// stepped-sine and noise probes; the simulation of this model matches the
// development renders to better than -100 dB residual):
//   * Detector: per-sample peak follower on |x|, env += a * (|x| - env), with
//     a = attack coefficient while |x| > env, release coefficient otherwise.
//     The coefficient is "tor" / (time in samples), with the machine's
//     Attack/Release "tor" attributes at their default 5 (i.e. the stated time
//     spans five time constants). The updated env is used for the same sample.
//   * Static curve in the linear amplitude domain: above threshold T the
//     output level is T + (env - T) / ratio, i.e.
//       gain = 1/R + (T / env) * (1 - 1/R)   when env > T, else 1.
//     So gain tends to 1/ratio for loud input (verified: steady state at
//     threshold -64 dB saturates at exactly -20*log10(ratio)).
//   * Output gain applied after the compressor.
// Modes: 0 "simple" (self-keyed, also sends to the internal bus), 1 "audio"
// (keyed from the internal bus channel), 2 "level" (level from the bus).
// masterblaster has no internal bus, so mode 1 is keyed by silence (= plain
// output gain, as the oracle shows when the bus is empty) and mode 2 outputs
// silence (every mode-2 oracle render is all zeros). Channel only selects the
// bus and has no audible effect here.
//
// Known differences: a few oracle renders show cross-job leftovers on the
// internal bus (mode 1 compressing a constant amount; mode 0 envelopes
// jumping to "infinite" at sample 128/163 so gain = 1/ratio). They are not
// modelled. Buzz's work_returns_false silence handling is not modelled.
//
// 2-in/2-out: the detector runs on (L+R)/2 and the same gain scales both.
import("stdfaust.lib");

mode      = rint(hslider("[0]Mode", 0, 0, 2, 1));
channel   = rint(hslider("[1]Channel", 1, 1, 32, 1));
threshold = hslider("[2]Threshold", 47512, 0, 65534, 1);
ratio     = hslider("[3]Ratio", 0, 0, 65534, 1);
attack    = hslider("[4]Attack", 12577, 0, 65534, 1);
release   = hslider("[5]Release", 7452, 0, 65534, 1);
gain      = hslider("[6]Gain", 0, 0, 65534, 1);

TOR = 5.0;
unit(v) = v / 65534.0;

thresh_lin = ba.db2linear(-64.0 + 80.0 * unit(threshold));
inv_ratio  = 1.0 - unit(ratio) * (1.0 - 1.0 / 25.0);
attack_ms  = 0.1 + 199.9 * unit(attack);
release_ms = 50.0 + 2950.0 * unit(release);
out_gain   = ba.db2linear(64.0 * unit(gain));

ms2coef(ms) = TOR / (ms * 0.001 * ma.SR);

// Peak follower: first input is the fed-back previous envelope.
follow(prev, d) = prev + select2(d > prev, ms2coef(release_ms), ms2coef(attack_ms)) * (d - prev);
envelope = abs : (follow ~ _);

curve(env) = select2(env > thresh_lin, 1.0,
                     inv_ratio + thresh_lin / max(env, 1e-30) * (1.0 - inv_ratio));

// Mode 1 is keyed from an (absent) bus: silent key, unity compressor gain.
key(l, r) = (l + r) * 0.5 * (mode == 0);
mode_gain = out_gain * (mode != 2);

process(l, r) = l * g, r * g
with {
    g = key(l, r) : envelope : curve : *(mode_gain) : attach(_, channel);
};
