// Muon Delay (Dave Waugh) — clean-room port, derived only from info.json,
// describe.json, triage.json and the buzz-machines-oracles measurements
// (impulse / pink-noise / burst probes). No machine source was read.
//
// Signal path (mono, run on both channels):
//   s[n]   = x[n] + fb * wet[n]                 (buffer write)
//   wet[n] = LPF( s[n - D] )                     (integer tap, then lowpass)
//   out    = dry * x + wet * wet
// The lowpass sits inside the feedback loop: echo k is fb^(k-1) * H^k.
//
// Parameter mappings (from describe.json, confirmed by impulse fits):
//   Delay     D = ms * 44 samples (exact integer tap; 480 ms -> 21120)
//   Feedback  fb = v/255 - 0.5   (-0.50 .. +0.50, default 128 -> +0.00196)
//   Wet, Dry  v/255
//   Cutoff    RBJ-style 2-pole lowpass, numerator (1,2,1), unit DC gain;
//             f = exp(3.40125 + 0.0499742*c) Hz (30 Hz .. 17.1 kHz, fit of
//             pole angles at c = 0,16,32,64,95,127 — above Nyquist/2 the
//             poles wrap exactly as the RBJ formulas predict).
//             Fitted Q is piecewise constant: 1.000 (c 0,16,20), 1.1454
//             (c 32,33,35,64,91), 1.3581 (c 95,119,127). Every measured
//             cutoff lands on one of the three values, so this is modelled as
//             a step; the boundaries (c < 26, c < 93) are midpoints between
//             measured cutoffs — the exact boundaries are unknown.
//   LFO Depth A = v * 882/127 samples, LFO Freq f = 0.01 + v*1.99/127 Hz.
//             When depth > 0 the Delay parameter is ignored and the tap is
//             D = max(1, floor(A/2 * (1 + sin(2*pi*k/2048)))), with
//             k = floor(2048 * phase): a 2048-entry sine table without
//             interpolation (delay steps land on a grid of 44100/(2048 f)
//             samples in pink-noise step timing, e.g. 63.51 samples at
//             0.339 Hz). Phase 0 at render start.
//             The phase only advances in host blocks with non-silent input;
//             in silent blocks the last tap is held (impulse probe: frozen
//             echo spacing 472 for depth/freq 127/127, and all 17 measured
//             first-echo delays fit 255 phase steps; burst probe: tap frozen
//             at the value for the block containing the burst end).
//   Known differences: tick length is assumed to be 5250 samples for the
//   silence grid; the activity test is causal (the first samples of a block
//   before its first non-zero input still use the held tap); a slow phase
//   drift of ~4e-7 table steps/sample seen in the oracle is not modelled.
//   Mono machine: the same path runs on both channels.
import("stdfaust.lib");

delayMs   = rint(hslider("[0]Delay", 480, 1, 2000, 1));
feedback  = rint(hslider("[1]Feedback", 128, 0, 255, 1));
cutoff    = rint(hslider("[2]Cutoff", 16, 0, 127, 1));
lfoDepth  = rint(hslider("[3]LFO Depth", 0, 0, 127, 1));
lfoFreq   = rint(hslider("[4]LFO Freq", 0, 0, 127, 1));
wetGain   = rint(hslider("[5]Wet", 128, 0, 255, 1)) / 255.0;
dryGain   = rint(hslider("[6]Dry", 128, 0, 255, 1)) / 255.0;

MAXDELAY = 131072;

fbGain = feedback / 255.0 - 0.5;

// ---- lowpass (RBJ biquad) ----
cutoffHz(c) = exp(3.40125 + 0.0499742 * c);
cutoffQ(c) = ba.if(c < 26, 1.0, ba.if(c < 93, 1.1454, 1.35811));
lowpass = fi.tf2(b0, 2.0 * b0, b0, a1, a2)
with {
    f = cutoffHz(cutoff);
    w = 2.0 * ma.PI * f / 44100.0;
    alpha = sin(w) / (2.0 * cutoffQ(cutoff));
    a0 = 1.0 + alpha;
    b0 = (1.0 - cos(w)) / 2.0 / a0;
    a1 = -2.0 * cos(w) / a0;
    a2 = (1.0 - alpha) / a0;
};

// ---- delay length ----
// LFO: 2048-entry sine table read without interpolation; the phase
// accumulator only advances while the host block's input is non-silent
// (Buzz passes silent blocks without WM_READ). Blocks are 256 samples on a
// grid that restarts every tick (5250 samples at 126 BPM / 4 TPB; Faust gets
// no tempo, so the default tick length is assumed). The silence test is
// causal: a block counts as active from its first non-zero input sample,
// and the phase then catches up to the block start.
TICK = 5250;
BLOCK = 256;
PHASE_MOD = 560070000; // 12700 * 44100: f = (127 + 199 v) / 12700 Hz

sampleIndex = (+(1) ~ _) - 1;
blockPos = (sampleIndex % TICK) % BLOCK;
blockStart = blockPos == 0;

// 1 once a non-zero input sample has been seen in the current block.
activeInBlock(x) = (x != 0, blockStart) : (keep ~ _)
with { keep(prev, nz, start) = nz | (prev & (1 - start)); };

// Phase accumulator (integer, mod PHASE_MOD) at the current block's start.
phaseBase(active) = (blockStart, active', blockPos') : (advance ~ _)
with {
    advance(prev, start, wasActive, lastPos) =
        (prev + start * wasActive * (lastPos + 1) * lfoStep) % PHASE_MOD;
};
lfoStep = int(127 + 199 * lfoFreq);
lfoPhase(active) = ((phaseBase(active) + blockPos * lfoStep) % PHASE_MOD)
                   / float(PHASE_MOD);
tableSin(ph) = sin(2.0 * ma.PI * floor(ph * 2048.0) / 2048.0);
lfoTap(active) = floor(lfoDepth * 441.0 / 127.0 * (1.0 + tableSin(lfoPhase(active))));

// In silent blocks the last computed LFO tap is held.
holdWhen(on, v) = (on, v) : (pick ~ _)
with { pick(prev, o, nv) = select2(o, prev, nv); };
lfoSamples(x) = holdWhen(activeInBlock(x), lfoTap(activeInBlock(x)));
tapSamples(x) = max(1, ba.if(lfoDepth > 0, lfoSamples(x), delayMs * 44));

// x is delayed by one sample so that, together with the implicit one-sample
// delay of `~`, both input and feedback reach the tap after exactly D samples.
echoLoop(D) = (+ : de.delay(MAXDELAY, D - 1) : lowpass) ~ *(fbGain);
muon(x) = dryGain * x + wetGain * (x : mem : echoLoop(tapSamples(x)));

process = muon, muon;
