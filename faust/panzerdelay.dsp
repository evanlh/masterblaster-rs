// FSM PanzerDelay -- stereo delay with a shared-state resonant filter in the
// feedback loop, per-tap panning and a feedback spread / ping-pong matrix.
//
// Clean-room port derived only from the oracle's measurements (impulse and
// stereo-matrix probes) and the machine's own value text:
//  * Amplitudes (Dry amp, Tap Amp, FB Amp): gain = 2^((v-240)/60), v=0 -> 0
//    (describe.json: 120 -> -12.0 dB, 60 -> -18.0 dB; echoes measured at
//    exactly 1/2, 1/4, 1/8 per 60 steps).
//  * Pans (0..240): equal-power sqrt law, L gain = sqrt((240-p)/240),
//    R gain = sqrt(p/240) (measured 0.866 / 0.5 at 60, 0.707 at 120).
//    "Dry L Pan"/"Wet L Pan" place the LEFT channel (line) in the stereo
//    field, "... R Pan" the right one.
//  * Delay: Ticks = len*5250 samples (126 BPM, 4 TPB, 44.1 kHz; the oracle
//    host's tick; Faust has no tempo), Ticks/256, Samples, ms. The wet
//    path is exactly len samples long (echo at +len). Measured limit:
//    65335 samples; measured minimum: 50 samples (Samples unit, len 3,
//    echoes every 50). The delay line is written from the start but is not
//    read (no tap, no feedback) during the first tick (5250 samples) after
//    start: with a 50-sample delay on noise the wet signal begins exactly
//    at sample 5250 and has no echo build-up from before it.
//  * Filter (None/LP/HP/BP): one RBJ biquad (LP/HP/constant-0dB-peak BP)
//    whose state is SHARED between the two delay lines: the left line's
//    sample and then the right line's sample pass through the same state
//    each frame (an interleaved stream at 2x rate). It sits before the delay
//    write, inside the feedback loop. f = 440*(8000/440)^(cutoff/240) Hz
//    (w0 = 2*pi*f/44100), Q = 0.01 + 0.99*res/240; all fitted from the five
//    cutoff/resonance points the oracle exercises and checked with the
//    biquad numerators (LP a(1,2,1), HP g(1,-2,1), BP k(1,0,-1)).
//  * Feedback: gain fb, then a spread matrix [[1-|d|, d],[d, 1-|d|]] with
//    d=(120-spread)/240 (spread 120 = identity, 0 = half crossfeed, 240 =
//    anti-phase 0.5/-0.5), then optional channel swap (ping-pong).
import("stdfaust.lib");


// masterblaster maps raw -> 0..65535 -> slider by truncation: the slider can
// sit up to (range/65535) below the raw value, so round back to the raw integer.
raw(x) = floor(x + 0.5);

dryAmp   = raw(hslider("[00]Dry amp", 240, 0, 240, 1));
dryLPan  = raw(hslider("[01]Dry L Pan", 0, 0, 240, 1));
dryRPan  = raw(hslider("[02]Dry R Pan", 240, 0, 240, 1));
unit     = raw(hslider("[03]Delay unit", 0, 0, 3, 1));
ftype    = raw(hslider("[04]Filter type", 0, 0, 3, 1));
cutoff   = raw(hslider("[05]Cutoff", 120, 0, 240, 1));
reso     = raw(hslider("[06]Resonance", 120, 0, 240, 1));
lenL     = raw(hslider("[07]L Delay Len", 3, 1, 20000, 1));
lenR     = raw(hslider("[08]R Delay Len", 3, 1, 20000, 1));
tapAmp   = raw(hslider("[09]Tap Amp", 240, 0, 240, 1));
wetLPan  = raw(hslider("[10]Wet L Pan", 240, 0, 240, 1));
wetRPan  = raw(hslider("[11]Wet R Pan", 0, 0, 240, 1));
fbAmp    = raw(hslider("[12]FB Amp", 150, 0, 239, 1));
pingpong = raw(hslider("[13]FB PingPong", 0, 0, 1, 1));
spread   = raw(hslider("[14]FB Spread", 120, 0, 240, 1));

// ---- parameter -> value maps ----
gain(v) = ba.if(v == 0, 0, pow(2.0, (v - 240.0) / 60.0));
panL(v) = sqrt((240.0 - v) / 240.0);
panR(v) = sqrt(v / 240.0);

tickSamples = 5250.0;
maxDelay = 65335;
minDelay = 50;
delaySamples(len) = int(max(minDelay, min(maxDelay, ba.selectn(4, unit,
    len * tickSamples, floor(len * tickSamples / 256.0), len, floor(len * ma.SR / 1000.0)))));

// ---- shared-state biquad ----
w0 = 2.0 * ma.PI * 440.0 * pow(8000.0 / 440.0, cutoff / 240.0) / 44100.0;
cw = cos(w0);
q = 0.01 + 0.99 * reso / 240.0;
al = sin(w0) / (2.0 * q);
a0 = ba.if(ftype == 0, 1.0, 1.0 + al);
nb0 = ba.selectn(4, ftype, 1.0, (1.0 - cw) / 2.0, (1.0 + cw) / 2.0, al) / a0;
nb1 = ba.selectn(4, ftype, 0.0, 1.0 - cw, 0.0 - (1.0 + cw), 0.0) / a0;
nb2 = ba.selectn(4, ftype, 0.0, (1.0 - cw) / 2.0, (1.0 + cw) / 2.0, 0.0 - al) / a0;
na1 = ba.if(ftype == 0, 0.0, -2.0 * cw / a0);
na2 = ba.if(ftype == 0, 0.0, (1.0 - al) / a0);

biquadStep(x, s1, s2) = y, n1, n2
with {
    y = nb0 * x + s1;
    n1 = nb1 * x - na1 * y + s2;
    n2 = nb2 * x - na2 * y;
};
frame(s1, s2, a, b) = s1c, s2c, ya, yb
with {
    ya = nb0 * a + s1;
    s1a = nb1 * a - na1 * ya + s2;
    s2a = nb2 * a - na2 * ya;
    yb = nb0 * b + s1a;
    s1c = nb1 * b - na1 * yb + s2a;
    s2c = nb2 * b - na2 * yb;
};
sharedFilter = (frame ~ (_, _)) : (!, !, _, _);

// ---- first tick gate ----
firstTickDone = (ba.time >= tickSamples);

// ---- feedback matrix ----
dsp_d = (120.0 - spread) / 240.0;
own = 1.0 - abs(dsp_d);
cross = dsp_d;
fbGain = gain(fbAmp) * firstTickDone;
spreadMix(a, b) = fbGain * (own * a + cross * b), fbGain * (cross * a + own * b);
pingMix(a, b) = ba.if(pingpong > 0.5, b, a), ba.if(pingpong > 0.5, a, b);
fbMatrix = pingMix : spreadMix;

// ---- lines ----
nL = delaySamples(lenL);
nR = delaySamples(lenR);
lineL = de.delay(65536, nL - 1);
lineR = de.delay(65536, nR - 1);
wet = (ro.interleave(2, 2) : (+, +) : sharedFilter : (lineL, lineR)) ~ fbMatrix;
tapGain = gain(tapAmp) * firstTickDone;
tap = (mem, mem) : (*(tapGain), *(tapGain));

// ---- pans and mix ----
panMix(pl, pr, a, b) = panL(pl) * a + panL(pr) * b, panR(pl) * a + panR(pr) * b;
wetPan(a, b) = panMix(wetLPan, wetRPan, a, b);
dryPath(l, r) = panMix(dryLPan, dryRPan, l, r) : (*(gain(dryAmp)), *(gain(dryAmp)));

process(l, r) = (dryPath(l, r), wetPath(l, r)) :> (_, _)
with { wetPath(l, r) = (l, r) : wet : tap : wetPan; };
