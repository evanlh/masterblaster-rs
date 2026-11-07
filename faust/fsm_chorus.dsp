// FSM Chorus (Krzysztof Foltman) -- clean-room Faust port.
//
// Derived only from info.json / describe.json / presets.json / triage.json and
// the buzz-machines-oracles measurements (no machine source was read):
//
// - Dry out: describe text reads (raw-240)/10 dB, but the fitted linear gain
//   is exactly 2^((raw-240)/60) (-6.0206 dB at 180, -18.062 dB at 60); raw 0
//   is silence. Wet out uses the same law (least-squares fit of the
//   dry and wet paths on pink noise, residual < -125 dB).
// - Wet path: one mono delay line, linear interpolation (2-tap least-squares
//   fits on noise show adjacent taps summing to 1.0).
// - Delay in samples = 4.41 * (MinDelay + Modulation * (1 + sin(2*pi*phase))/2),
//   i.e. raw units of 0.1 ms; the LFO is a true sine starting at phase 0
//   (sine model nulls the defaults job to -125 dB, a parabolic one fails).
// - LFO rate f = 0.05 * 800^(raw/255) Hz, phase continuous across rate changes.
// - Feedback = (raw-64)/64 applied as line_in = x + fb * tap (recursive sim
//   nulls -100%..+100% jobs to < -120 dB; no x0.995 scaling).
// - Parameter changes take effect immediately (automation jump/ramp jobs null
//   to > 100 dB with step changes on tick boundaries): no glides.
// Buzz machine is mono: the input is (L+R)/2 and both outputs carry the result.
import("stdfaust.lib");

raw(v) = floor(v + 0.5); // undo masterblaster's raw->0..65535->slider truncation

dry_raw  = hslider("[0]Dry out", 240, 0, 240, 1) : raw;
mind_raw = hslider("[1]Minimum delay", 20, 1, 100, 1) : raw;
mod_raw  = hslider("[2]Modulation", 10, 1, 100, 1) : raw;
rate_raw = hslider("[3]LFO rate", 10, 0, 240, 1) : raw;
wet_raw  = hslider("[4]Wet out", 240, 0, 240, 1) : raw;
fb_raw   = hslider("[5]Feedback", 64, 0, 128, 1) : raw;

gain(r) = select2(r > 0, 0.0, pow(2.0, (r - 240.0) / 60.0));
lfo_hz = 0.05 * pow(800.0, rate_raw / 255.0);
// The oracle's LFO phase does not drift like a float32 accumulator (feedback
// +-100% jobs expose it), so the phase is kept as an exact coarse part on a
// 2^-20 grid plus a small remainder that carries into it.
GRID = 1048576.0;
on_grid(v) = floor(v * GRID) / GRID;
inc = lfo_hz / ma.SR;
inc_hi = on_grid(inc);
inc_lo = inc - inc_hi;
phase_step(h, l) = ma.frac(h + inc_hi + on_grid(l + inc_lo)), (l + inc_lo) - on_grid(l + inc_lo);
phase = phase_step ~ (_, _) : mem, mem : +;
lfo = sin(2.0 * ma.PI * phase);
samples_per_raw = ma.SR / 10000.0;
delay = samples_per_raw * (mind_raw + mod_raw * (1.0 + lfo) * 0.5);
fb = (fb_raw - 64.0) / 64.0;

MAXD = 2048;
// y[n] = line[n - delay + 1]; wet = y[n-1] = line[n - delay]; line = x + fb*wet
wet_path = (+ : de.fdelay(MAXD, delay - 1.0)) ~ *(fb) : mem;

chorus(x) = gain(dry_raw) * x + gain(wet_raw) * wet_path(x);
process = + : *(0.5) : chorus <: _, _;
