declare name "Jeskola Reverb";
declare author "masterblaster port (clean room)";
declare description "Clean-room model of Jeskola Reverb (Oskari Tammelin), derived from oracle measurements only";

// Clean-room port: no machine source code or disassembly was consulted. Everything below
// was recovered from the buzz-machines-oracles measurements (impulse probe, development set):
//
// * Dry Out / Rev Out / ER Out: linear raw/4096 (the impulse passes the dry path at exactly
//   raw/4096; describe.json shows 20*log10(raw/4096) dB).
// * Early reflections: six single-sample taps (delay in samples : gain) fitted to 1e-7:
//   877:0.868 1561:0.696 1716:0.540 1826:0.612 3083:0.628 3510:0.204.
// * Reverb (exactly LTI while input is present, fitted to ~3e-7 relative RMS):
//   [comb(1774, gA) + delay(363) comb(2137, gB)] -> allpass(351, 0.7) -> allpass(1097, 0.7)
//   -> delay(1774). Comb feedback gA = -10^(-1.2114535/t) with t = Rev Time (tenths of a
//   second, i.e. RT60 = t/10 s), gB = -0.8732394 * gA (ratio constant across every t fitted).
// * The ER and reverb sum is filtered by an RBJ Butterworth lowpass (Q = 1/sqrt 2) at
//   High Cutoff Hz (raw value is Hz); filtering after the sum was verified by commuting it.
// * Pre-Delay is dead: every Pre-Delay setting reproduces the zero-pre-delay response.
// * Silent input: after the input has been silent for t*8820 samples (2 x RT60), counted from
//   the end of the last block with input, the wet sum feeding the lowpass is muted from the
//   next 256-sample block on (block grid restarting each tick); the lowpass keeps ringing out.
//   Bounds measured: hold/t in (8809.6, 8834] samples from cut points at t = 1 and t = 10.

import("stdfaust.lib");

// --- parameters (raw Buzz units; rounding recovers the raw value from the 0..65535 rescale)
rawValue(v) = int(v + 0.5);

dryRaw    = hslider("[0]Dry Out", 4096, 0, 4096, 1) : rawValue;
revRaw    = hslider("[1]Rev Out", 768, 0, 4096, 1) : rawValue;
erRaw     = hslider("[2]ER Out", 0, 0, 4096, 1) : rawValue;
revTime   = hslider("[3]Rev Time", 10, 1, 64, 1) : rawValue;
preDelay  = hslider("[4]Pre-Delay", 0, 0, 100, 1) : rawValue;
highCut   = hslider("[5]High Cutoff", 5000, 100, 16384, 1) : rawValue;

level(raw) = raw / 4096.0;

// --- early reflections
erTap(d, g) = @(d) : *(g);
early = _ <: erTap(877, 0.868), erTap(1561, 0.696), erTap(1716, 0.540),
             erTap(1826, 0.612), erTap(3083, 0.628), erTap(3510, 0.204) :> _;

// --- reverb network
combA = 0.0 - 10.0 ^ (0.0 - 1.2114535 / revTime);
combB = -0.8732394 * combA;

comb(d, g) = + ~ (@(d - 1) : *(g));
allpass(d, g) = (+ ~ (@(d - 1) : *(g))) <: (*(0.0 - g), @(d)) :> _;

reverb = _ <: comb(1774, combA), (comb(2137, combB) : @(363)) :> _
       : allpass(351, 0.7) : allpass(1097, 0.7) : @(1774);

// --- RBJ Butterworth lowpass on the wet sum
rbjLowpass(f) = fi.tf2(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
with {
    w = 2.0 * ma.PI * f / ma.SR;
    cw = cos(w);
    alpha = sin(w) / (2.0 * sqrt(0.5));
    b0 = (1.0 - cw) / 2.0;
    b1 = 1.0 - cw;
    b2 = b0;
    a0 = 1.0 + alpha;
    a1 = -2.0 * cw;
    a2 = 1.0 - alpha;
};

// --- silent-input gate on host blocks (grid restarts with every compute call)
blockPos = step ~ _ with { step(prev) = ba.if(prev + 1 >= ma.BS@1, 0, prev + 1); };
newBlock = blockPos == 0;

blockLoud(x) = (\(prev).(max(prev * (1 - newBlock), x != 0))) ~ _;

holdSamples = revTime * 8820;
holdCount(loud) = (\(prev).(ba.if(loud, holdSamples, prev - 1))) ~ _;

gateOpen(x) = (\(prev).(max(ba.if(newBlock, cnt' > 0, prev), loud))) ~ _
with {
    loud = blockLoud(x);
    cnt = holdCount(loud);
};

// --- signal path
wetSum(x) = level(erRaw) * early(x) + level(revRaw) * reverb(x);
wet(x) = wetSum(x) * gateOpen(x) : rbjLowpass(highCut);
// Pre-Delay is dead in the original; `preDelay > -1` is always 1 and keeps the slider alive.
mono(x) = (level(dryRaw) * x + wet(x)) * (preDelay > -1);

process = + : *(0.5) : mono <: _, _;
