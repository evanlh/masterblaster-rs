declare name "FSM WahPro2";
declare description "Clean-room port of FSM WahMan Pro 2 (registry key FSM WahPro2)";

// Clean-room model, derived only from info.json/presets.json and the oracle's
// measurements (per-block least-squares fits of the rendered noise-pink and
// steady-sine outputs against their inputs). No machine source was read.
//
// Measured structure
// - Control rate: filter coefficients change once per 64-sample block on a grid
//   that restarts every tick (5250 samples at 126 BPM / 4 TPB, so the last block
//   of a tick is 2 samples). masterblaster gives Faust no tempo, so the tick is
//   fixed at 126 BPM / 4 TPB.
// - Control value x in "cutoff steps": f = 264 Hz * 2^(x/48) (48 steps/octave,
//   cutoff 0 = 264 Hz, 240 = 8448 Hz), capped at 20 kHz.
//   x = glide(cutoff + LFO); the glide is a slew limiter that moves at most
//   10*exp(-0.0375*inertia) steps per block, starting from x = 0.
// - LFO: phase in cycles, reset to SetPhase/128 when Set phase is sent.
//   rate 0..239: 0.05 Hz * exp(0.0262145 rate) (~0.05..27 Hz); 240..254: tempo
//   synced, measured 240=1, 243=4, 245=8, 247=16, 248=24, 250=32, 254=128 ticks
//   per cycle; the rest are filled in on the same 1, 1.5, 2 grid (249 is a
//   guess). Depth A = mod/2 steps. Shapes (p = phase in cycles, units of A):
//   0 sin; 2 3-4p; 3 square (+1 first half); 4 triangle from -1; 6 rounded
//   square (approximated by tanh); 8 (sin 2t + cos 3t)/2; 10 3 - floor(16p)/4;
//   12 1.5-2p+frac(8p)/2+frac(16p)/8; 14 2-2p-frac(8p)/2+frac(16p)/8;
//   16 sample&hold, 32 values per cycle in +-1/2 (the machine's random
//   generator is unknown, so the values differ).
//   From one tick each of the Shape-ramp automation job: 9 = -(10),
//   11 = -1.5+2p+frac(8p)/2-frac(16p)/8, 5 ~ clip(1.6 sin).
//   Not measurable, guessed: 1 = -(2), 13 by analogy with 11, 7 = 8 (closest
//   of the candidates at its one tick), 15 = sample&hold.
// - Filters (bilinear / RBJ forms fitted to < -140 dB residual):
//   LP section: analog e/(s^2 + d s + e) with rho = sqrt(1-1/Q), d = 2(1-rho),
//   e = 1/rho, K = tan(pi f / SR).  P = 1.01 + res/48.
//   0: LP(f, Q=P^2), gain 2/sqrt(P)
//   1: peak(f, poleQ 10, A^2 = 1.01 + res/8), gain 2/A
//   2: peak(f, poleQ 1 + res/6, A^2 = 32), gain 1.2/(7 - res/60)
//   3: LP(f, Q=P) * LP(0.7 f, Q=P), gain 0.7
//   4: peak(f) * peak(f/(1+res/240)), poleQ 1 + res/12, A^2 = 6 + res/30,
//      gain 0.32/(3 - res/120)
//   5: peak(f) * peak(2f/(7 - res/60)), poleQ 8, A^2 = 9, gain 0.4
//   6: formants F1 (270,400,800) F2 (2140,800,1150) Hz at x = 0,120,240
//      (linear between, extrapolated outside), poleQ 2 + res/48,
//      A^2 = 6 + res/24, gain 0.18
//   7: formants F1 (270,400,650) F2 (2140,1700,1080), poleQ 2 + res/56,
//      A^2 = 6 + res/16, gain 0.18
//   Sections run in the order listed (formants: F1 then F2), direct form I.
//   "peak" is an RBJ peaking EQ: pole alpha = sin(w)/(2 poleQ), zero alpha =
//   pole alpha * A^2.
// Known differences: the machine stops output when the input is silent
// (Work returns false); this port keeps ringing. LFO random values differ.

import("stdfaust.lib");

cutoff  = hslider("[0]Cutoff", 120, 0, 240, 1) : rint;
reso    = hslider("[1]Resonance", 120, 0, 240, 1) : rint;
ftype   = hslider("[2]Filter", 6, 0, 7, 1) : rint;
lrate   = hslider("[3]LFO rate", 30, 0, 254, 1) : rint;
lmod    = hslider("[4]LFO mod", 60, 0, 240, 1) : rint;
lshape  = hslider("[5]LFO Shape", 0, 0, 16, 1) : rint;
inertia = hslider("[6]Inertia", 10, 0, 60, 1) : rint;
setph   = hslider("[7]Set phase", 0, 0, 127, 1) : rint;

// ---- control grid ---------------------------------------------------------
tickLen = int(ma.SR * 60.0 / (126.0 * 4.0) + 0.5);
blockStart = ((ba.time % tickLen) % 64) == 0;

// ---- LFO ------------------------------------------------------------------
syncTicks(r) = (1, 2, 3, 4, 6, 8, 12, 16, 24, 28, 32, 48, 64, 96, 128)
             : ba.selectn(15, max(0, min(14, int(r) - 240)));
freeRate(r) = 0.05 * exp(r * 0.0262145); // ~ 0.05 Hz * 540^(r/240), fitted
lfoHz = select2(lrate >= 240, freeRate(lrate), ma.SR / (tickLen * syncTicks(lrate)));

// Phase = offset + k * increment, with the offset folded at every tick start so
// single-precision accumulation does not drift (rate changes land on ticks).
phaseReset = (setph != setph') | (ba.time == 0);
tickStart = (ba.time % tickLen) == 0;
phaseInc = lfoHz / ma.SR;
phaseState = loop ~ (_, _)
with {
    loop(off, k) = select2(phaseReset, select2(tickStart, off, folded), setph / 128.0),
                   select2(phaseReset | tickStart, k + 1, 0)
    with {
        folded = ma.frac(off + float(k + 1) * phaseInc');
    };
};
phase = phaseState : \(off, k).(ma.frac(off + float(k) * phaseInc));

saw(n, p) = ma.frac(n * p);
sinc(n, p) = sin(2.0 * ma.PI * n * p);
cosc(n, p) = cos(2.0 * ma.PI * n * p);

shapeSine(p)     = sinc(1, p);
shapeRamp(p)     = 3.0 - 4.0 * p;
shapeSquare(p)   = select2(p < 0.5, -1.0, 1.0);
shapeTriangle(p) = 1.0 - 4.0 * abs(p - 0.5);
shapeRounded(p)  = ma.tanh(2.2 * sinc(1, p)) / ma.tanh(2.2);
shapeHarm(p)     = 0.5 * (sinc(2, p) + cosc(3, p));
shapeSteps(p)    = 3.0 - 0.25 * floor(16.0 * p);
shapeSawsUp(p)   = 1.5 - 2.0 * p + 0.5 * saw(8, p) + 0.125 * saw(16, p);
shapeSawsDown(p) = 2.0 - 2.0 * p - 0.5 * saw(8, p) + 0.125 * saw(16, p);
shapeRiseA(p)    = 0.0 - 1.5 + 2.0 * p + 0.5 * saw(8, p) - 0.125 * saw(16, p);
shapeRiseB(p)    = 0.0 - 2.0 + 2.0 * p - 0.5 * saw(8, p) - 0.125 * saw(16, p);
shapeClipped(p)  = max(-1.0, min(1.0, 1.6 * sinc(1, p)));

holdTrig(p) = floor(32.0 * p) != floor(32.0 * p');
shapeHold(p) = ba.sAndH(holdTrig(p), 0.5 * no.noise);

shapeValue(s, p) = (shapeSine(p), 0.0 - shapeRamp(p), shapeRamp(p), shapeSquare(p),
                    shapeTriangle(p), shapeClipped(p), shapeRounded(p), shapeHarm(p),
                    shapeHarm(p), 0.0 - shapeSteps(p), shapeSteps(p), shapeRiseA(p),
                    shapeSawsUp(p), shapeRiseB(p), shapeSawsDown(p), shapeHold(p),
                    shapeHold(p)) : ba.selectn(17, s);

lfo = 0.5 * lmod * shapeValue(lshape, phase);

// ---- glide (slew limiter per block) ----------------------------------------
xMax = 48.0 * log(20000.0 / 264.0) / log(2.0);
xMin = 48.0 * log(20.0 / 264.0) / log(2.0);
slewStep = 10.0 * exp(-0.0375 * inertia);
target = max(xMin, min(xMax, cutoff + lfo));
glide = (\(g).(select2(blockStart, g, g + max(0.0 - slewStep, min(slewStep, target - g))))) ~ _;

freqOf(x) = 264.0 * pow(2.0, x / 48.0);

// ---- filter sections (b0, b1, b2, a1, a2) ----------------------------------
peakCoefs(f, poleQ, a2gain) = (1.0 + an) / n, -2.0 * c / n, (1.0 - an) / n,
                              -2.0 * c / n, (1.0 - ad) / n
with {
    w = 2.0 * ma.PI * min(f, 0.49 * ma.SR) / ma.SR;
    c = cos(w);
    ad = sin(w) / (2.0 * poleQ);
    an = ad * a2gain;
    n = 1.0 + ad;
};

lpCoefs(f, q) = b0, 2.0 * b0, b0, (2.0 * e * k2 - 2.0) / n, (1.0 - d * k + e * k2) / n
with {
    k = tan(ma.PI * min(f, 0.49 * ma.SR) / ma.SR);
    k2 = k * k;
    rho = sqrt(1.0 - 1.0 / q);
    d = 2.0 * (1.0 - rho);
    e = 1.0 / rho;
    n = 1.0 + d * k + e * k2;
    b0 = e * k2 / n;
};

unitCoefs = 1.0, 0.0, 0.0, 0.0, 0.0;

// direct form I; inputs: b0, b1, b2, a1, a2, signal
biquad(b0, b1, b2, a1, a2, sig) = (b0 * sig + b1 * sig' + b2 * sig'') : (+ ~ feedback)
with {
    feedback(y) = 0.0 - a1 * y - a2 * y';
};

formant(x, v0, v1, v2) = select2(x < 120.0, v1 + (v2 - v1) * (x - 120.0) / 120.0,
                                            v0 + (v1 - v0) * x / 120.0);

// ---- per-type parameters ----------------------------------------------------
P = 1.01 + reso / 48.0;
f = freqOf(glide);
x = glide;

pick8(t, a0, a1, a2, a3, a4, a5, a6, a7) = (a0, a1, a2, a3, a4, a5, a6, a7) : ba.selectn(8, t);

firstSection = (
    lpCoefs(f, P * P),
    peakCoefs(f, 10.0, 1.01 + reso / 8.0),
    peakCoefs(f, 1.0 + reso / 6.0, 32.0),
    lpCoefs(f, P),
    peakCoefs(f, 1.0 + reso / 12.0, 6.0 + reso / 30.0),
    peakCoefs(f, 8.0, 9.0),
    peakCoefs(formant(x, 270.0, 400.0, 800.0), 2.0 + reso / 48.0, 6.0 + reso / 24.0),
    peakCoefs(formant(x, 270.0, 400.0, 650.0), 2.0 + reso / 56.0, 6.0 + reso / 16.0)) : ba.selectbus(5, 8, ftype);

secondSection = (
    unitCoefs,
    unitCoefs,
    unitCoefs,
    lpCoefs(0.7 * f, P),
    peakCoefs(f / (1.0 + reso / 240.0), 1.0 + reso / 12.0, 6.0 + reso / 30.0),
    peakCoefs(2.0 * f / (7.0 - reso / 60.0), 8.0, 9.0),
    peakCoefs(formant(x, 2140.0, 800.0, 1150.0), 2.0 + reso / 48.0, 6.0 + reso / 24.0),
    peakCoefs(formant(x, 2140.0, 1700.0, 1080.0), 2.0 + reso / 56.0, 6.0 + reso / 16.0)) : ba.selectbus(5, 8, ftype);

outGain = pick8(ftype, 2.0 / sqrt(P), 2.0 / sqrt(1.01 + reso / 8.0), 1.2 / (7.0 - reso / 60.0),
                0.7, 0.32 / (3.0 - reso / 120.0), 0.4, 0.18, 0.18);

wah(sig) = (secondSection, (firstSection, sig : biquad)) : biquad : *(outGain);

process = wah, wah;
