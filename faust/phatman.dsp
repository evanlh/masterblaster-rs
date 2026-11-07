// FSM PhatMan: stereo chorus. Clean-room port derived only from the
// buzz-machines-oracles measurements (info.json, describe.json, oracle renders).
// No machine source code was read.
//
// Signal path (each part measured; see the numbers below):
// - The input is summed to mono, (L+R)/2, and fed into ONE delay line. The
//   stereo-matrix probe shows an L-only input reaching both outputs with
//   half-amplitude taps.
// - There are two linearly interpolated taps, L and R. Per-block 2-tap
//   least-squares fits give weights (1-frac, frac).
// - Delay = Min + Depth * u(phase), where Min = 0.1 ms * raw (4.41 samples)
//   and Depth = 0.1 ms * raw. The fitted minimum delays are 4.41*v exactly.
// - The LFO shape is a parabolic "sine": two parabolas per cycle (shape 4,
//   "phat"). A per-sample delay fit from the 440 Hz phase gives 0.002-0.02
//   samples rms, against 0.75 samples for a true sine.
//   u = 0.5 - 0.5*para(phase): the L tap starts mid-depth and moves down.
// - The LFO rate is f = 0.05 * 800^(raw/255) Hz. Fits at raw 0/10/64/127/191
//   agree to <0.01 %. Raw 254 measured exactly 1/16 Hz (period 16 s) in both
//   the one-at-a-time and the automation renders; 240 is fast. Only these two
//   points exist above 239, so 254 is special-cased and the rest of 240..253
//   stays on the exponential curve.
// - R phase = L phase + 0.5 + Phasing/128 cycles. Fits at Phasing
//   0/32/64/96/128 match.
// - Reset Phase v (an event) sets the LFO phase to v/240 cycles.
// - Glide ("inertia"): each tap's actual delay is target - e, where e
//   collects only the discontinuous jumps of the target (Min, Depth,
//   Phasing, Reset changes, and the start from 0) and decays by
//   (31/32)^(1/64) per sample (tau ~2016 samples = 31/32 per 64-sample block).
//   The free-running LFO motion itself is not smoothed: at rate 191
//   (7.47 Hz) the delay swing is full with no lag, while jumps glide with
//   tau ~2000 in the delay domain.
// - Feedback = (raw-64)/64 * 0.995 of the tap average (L+R)/2 into the line.
//   Simulations of fb=0/32/96/128 best match a magnitude about 0.5 % below
//   unity. That fit is the evidence for the 0.995.
// - Dry/Wet gain = 2^((raw-240)/60), with 0 = mute. Measured exactly 0.5,
//   0.25 and 0.125 at 180/120/60, although describe.json prints 0.1 dB steps.
// The machine returns false on silent 256-sample input blocks, but none of
// the oracle probes ever has an all-silent input block, so this port does
// not model the cut.
import("stdfaust.lib");

dry_p  = hslider("[0]Dry out", 240, 0, 240, 1);
rst_p  = hslider("[1]Reset Phase", 0, 0, 240, 1);
mind_p = hslider("[2]Minimum delay", 20, 1, 100, 1);
mod_p  = hslider("[3]Modulation", 10, 1, 100, 1);
rate_p = hslider("[4]LFO rate", 10, 0, 254, 1);
wet_p  = hslider("[5]Wet out", 240, 0, 240, 1);
fb_p   = hslider("[6]Feedback", 64, 0, 128, 1);
phs_p  = hslider("[7]Phasing", 32, 0, 128, 1);

raw(s) = floor(s + 0.5);              // recover integer Buzz value
gain(v) = select2(v > 0, 0.0, pow(2.0, (v - 240.0) / 60.0));

POLE = pow(31.0 / 32.0, 1.0 / 64.0);
PSCALE = 1073741824;                   // 2^30 fixed-point LFO phase

lfo_hz(r) = select2(r >= 254, 0.05 * pow(800.0, r / 255.0), 1.0 / 16.0);
inc = int(lfo_hz(raw(rate_p)) / ma.SR * PSCALE + 0.5);
acc_int = ((_ + inc) % PSCALE) ~ _ : mem;
acc = float(acc_int) / PSCALE;

rst = raw(rst_p);
reset = rst != rst';
off = (\(prev).(select2(reset, prev, ma.frac(rst / 240.0 - acc + 1.0)))) ~ _;

para(ph) = select2(x < 2.0, (x - 3.0) * (x - 3.0) - 1.0, 1.0 - (x - 1.0) * (x - 1.0))
with { x = ma.frac(ph) * 4.0; };
target(m, d, ph) = m + d * (0.5 - 0.5 * para(ph));

M = 4.41 * raw(mind_p);
D = 4.41 * raw(mod_p);
chofs = 0.5 + raw(phs_p) / 128.0;

// glide residual: e[n] = POLE * (e[n-1] + jump[n])
glide(j) = (\(prev).(POLE * (prev + j))) ~ _;
tapdelay(sh) = target(M, D, ph) - glide(target(M, D, ph) - target(M', D', php))
with {
  ph  = acc + off + sh;
  php = acc + off' + sh';
};
dL = tapdelay(0.0);
dR = tapdelay(chofs);

MAXD = 2048;
fb = (raw(fb_p) - 64.0) / 64.0 * 0.995;
rd(d, w) = de.fdelay(MAXD, max(0.0, d), w);
line(mono) = (\(wp).(mono + fb * 0.5 * (rd(dL - 1.0, wp) + rd(dR - 1.0, wp)))) ~ _;

process(l, r) = gain(raw(dry_p)) * l + gain(raw(wet_p)) * rd(dL, w),
                gain(raw(dry_p)) * r + gain(raw(wet_p)) * rd(dR, w)
with { w = line(0.5 * (l + r)); };
