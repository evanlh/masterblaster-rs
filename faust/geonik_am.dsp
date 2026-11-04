// Geonik's Amplitude Modulation — mono-in, stereo-out LFO tremolo.
//
// Clean-room port: derived only from info.json / describe.json and the
// buzz-machines-oracles measurements (sample gain = output / input on the
// noise-pink and steady-sine probes). No machine source was read.
//
// Measured model:
//  * The LFO is a 128-step staircase: gain changes exactly when
//    floor(n * f * 128 / SR) changes (n = samples since start), with
//    f = Frequency / 1000 Hz (describe.json: "1.000Hz" at 1000). The step
//    values are the waveform evaluated at integer step e in 0..127, so the
//    staircase is the original's resolution (fine time steps at 1 Hz and
//    16.384 Hz show the same value sequence, one entry per step).
//  * Gain = floor + (1 - floor) * lfo(e), floor = Floor / 128 ("100.0%" at 128).
//  * Waveforms, lfo(e) in 0..1, measured entry by entry:
//      0 Sine        0.5 + 0.5 sin(2 pi e / 128)
//      1 Triangle    0.5 at e=0, 1 at 32, 0 at 96 (slope 1/64 per step)
//      2 Pulse       trapezoid, slopes 1/8 per step: 0.5 + e/8 (e<4), 1 up to 60,
//                    falls to 0 at 68, 0 up to 125, then 0.125, 0.25 (e=126,127)
//      3 Saw         1 - e/124 for e <= 124, then (e-124)/4 back up
//      4 ReverseSaw  1 - Saw
//  * Right channel reads step (e + 64 - StereoPhase) mod 128 (StereoPhase 64 =
//    in phase, 0/128 = half a cycle). triage.json lists StereoPhase as dead,
//    but the oracle's two output channels clearly differ with it.
//  * Phase starts at 0, runs continuously across ticks and frequency changes;
//    Restart = 1 (switch, delivered 0/1) resets the phase to 0. Restart = 0
//    does not (automation:toggle:Restart).
//
// Word-parameter scaling: see unscaleWord (masterblaster's integer 0..65535
// rescale truncates, which costs a whole Hz step at high Frequency values).
// Single precision: the phase is kept as base + count * rate and rebased every
// 1024 steps, so floor(n * rate) is reproduced to well under a sample.
// Known difference: the original stops output on silent input
// (work_returns_false); the Faust path keeps running (output is 0 anyway).
import("stdfaust.lib");

// masterblaster delivers raw r as v = floor(r * 65535 / 65534), slider = v * 65534 / 65535,
// i.e. r - r/65535: plain rint would land one below for r > 32767. Undo the scaling.
frequency = hslider("[0]Frequency", 1000, 0, 65534, 1) : unscaleWord;
unscaleWord(s) = min(rint(s * 65535.0 / 65534.0), 65534);
lfoType = hslider("[1]LfoType", 0, 0, 4, 1) : rint;
floorVol = hslider("[2]Floor", 64, 0, 128, 1) : rint;
stereoPhase = hslider("[3]StereoPhase", 64, 0, 128, 1) : rint;
restart = hslider("[4]Restart", 0, 0, 1, 1) : rint;

STEPS = 128;
REBASE = 1024;

stepRate = frequency / 1000.0 * STEPS / ma.SR;
restartEdge = restart > restart';

// One sample of the phase state (base, count): phase = base + count * rate.
phaseStep(rate, trig, b, c) = nb, nc
with {
  prev = rate';
  cont = b + (c + 1) * prev;
  rebase = (rate != prev) | ((c + 1) * prev >= REBASE);
  nb = select2(trig, select2(rebase, b, ma.modulo(cont, STEPS)), 0);
  nc = select2(trig | rebase, c + 1, 0);
};

phaseOf(rate, b, c) = b + c * rate;

lfoStep = (phaseStep(stepRate, restartEdge) ~ (_, _)) : phaseOf(stepRate)
        : floor : int : %(STEPS);

wrapStep(e) = (e + STEPS) % STEPS;

triangle(e) = select2(d < 32, (64 - d) / 32.0, d / 32.0)
with { d = wrapStep(e + 32) - 32; };

sine(e) = 0.5 + 0.5 * sin(2.0 * ma.PI * e / STEPS);
tri(e) = 0.5 + 0.5 * triangle(e);
pulse(e) = select2(e < 4, select2(e <= 60, select2(e < 68, select2(e <= 125,
             (e - 125) / 8.0, 0.0), 1.0 - (e - 60) / 8.0), 1.0), 0.5 + e / 8.0);
saw(e) = select2(e <= 124, (e - 124) / 4.0, 1.0 - e / 124.0);
revSaw(e) = 1.0 - saw(e);

lfo(e) = ba.selectn(5, lfoType, sine(e), tri(e), pulse(e), saw(e), revSaw(e));

floorGain = floorVol / 128.0;
gainAt(e) = floorGain + (1.0 - floorGain) * lfo(e);

rightStep(e) = wrapStep(e + 64 - int(stereoPhase));

gains = lfoStep <: gainAt, (rightStep : gainAt);

process(l, r) = ((l + r) * 0.5) <: _, _ : (_, _), gains : ro.interleave(2, 2) : *, *;
