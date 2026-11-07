// Geonik's Gate — downward expander / noise gate (clean-room port)
//
// Derived only from info.json/describe.json and oracle measurements
// (buzz-machines-oracles, slug geonik-s-gate); no machine source was read.
//
// Measured structure:
// - Detector: one-pole mean-square of the input, time constant ~690 samples
//   (fit on tone-burst onsets; independent of Attack/Release), L = sqrt(P).
// - Threshold: linear T = 2^((raw-128)/12)  (6 dB per doubling; the
//   displayed "-64..0 dB" uses 0.5 dB/step but the 2^x form fits the
//   steady-state gains to 0.3% across five thresholds).
// - Ratio R = 1 + raw/10; raw 240 = "Noise Gate", which continues the line (R = 25); R=25 scored
//   better than a near-infinite ratio on noise-pink.
// - Static curve (linear, verified with R=3 and R=7 at the same detector
//   state): g = clamp(R - (R-1)*T/L, 0, 1).
// - Gain smoothing: one-pole toward the target, time constant Attack/5 ms
//   when opening, Release/5 ms when closing (fit tau = A/5 to 0.5%).
// - Work returns false on silent input: on all-silent blocks (256-sample grid
//   restarting each tick = each compute() call here) the output is zero
//   and detector/gain state is frozen (bursts after silence resume from the
//   pre-silence state). Emulated causally: state frozen while the current
//   block has been silent so far.
// - The original computes the target per block at block end (lookahead) and
//   ramps the gain linearly over the block; this port updates per sample.
import("stdfaust.lib");

// masterblaster rescales raw -> 0..65535 -> slider range with truncation,
// so recover the integer raw value by rounding.
rawInt = floor(_ + 0.5);
thr = hslider("[0]Threshold", 92, 0, 128, 1) : rawInt;
rat = hslider("[1]Ratio", 20, 0, 240, 1) : rawInt;
att = hslider("[2]Attack", 10, 5, 8000, 1) : rawInt;
rls = hslider("[3]Release", 2000, 30, 8000, 1) : rawInt;

T = pow(2.0, (thr - 128.0) / 12.0);
R = 1.0 + rat / 10.0; // raw 240 ("Noise Gate") gives 25:1
coefMs(ms) = 1.0 - exp(-5000.0 / (ms * ma.SR));
cA = coefMs(att);
cR = coefMs(rls);
detA = exp(-1.0 / 690.0);

// position within the current compute() block
nextPos(p) = select2(p + 1 >= ma.BS, p + 1, 0);
pos = nextPos ~ _ : mem;

// true while every sample of this block so far was zero
silentSoFar(x) = (\(s).((x == 0) & ((pos == 0) | s))) ~ _;

hold(frz, v, nv) = select2(frz, nv, v);
detStep(frz, x, p) = hold(frz, p, x * x + detA * (p - x * x));
target(P) = max(0.0, min(1.0, R - (R - 1.0) * T / max(sqrt(P), 1e-20)));
gainStep(frz, t, g) = hold(frz, g, g + select2(t > g, cR, cA) * (t - g));

gate(x) = gain
with {
    frz = silentSoFar(x);
    P = detStep(frz, x) ~ _;
    gain = gainStep(frz, target(P)) ~ _;
};

process(l, r) = l * g, r * g with { g = gate((l + r) * 0.5); };
