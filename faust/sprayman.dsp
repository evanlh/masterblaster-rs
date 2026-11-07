// FSM SprayMan -- clean-room granular "spray" delay (mono in, stereo out).
// Derived only from oracle measurements (no machine source read):
//  * Dry/Wet gain: exactly 2^((v-240)/60) (fits 0.125/0.25/0.5 at 60/120/180), v=0 -> silence.
//  * Grains read the (input + feedback) history at a random delay d0, uniform in
//    [85, 85+D(scatter)], D = 263 + 92*2^(S/9.33) samples (lag percentiles at S=0..64).
//  * Grain envelope: linear attack/sustain/release, each 309*2^(v/44) samples
//    (half-max widths of decoded grains: A0 S240 R0 ~ 14k, A60 S120 R32 ~ 3.3k).
//  * Per-grain linear pan p = (1-s)*pan/240 + s*(0.5 + 0.65*(u1+u2-1)), s = spread/100
//    (per-grain L/R ratios compared across Pan/Spread configs; L+R ~ constant).
//  * Fatness>0 gives small random rate deviations (~0.3-0.6%), Fatness=0 reads
//    integer delays (no interpolation HF loss, as measured).
//  * Fullness picks octave-down (rate 1/2, 1/4) grains: P0 = exp(-F/105), P1 <= 0.21.
//  * Feedback: echo of each grain at twice its lag with ratio 0.14 at Feedback 99.
//  * Richness = voices; per-grain gain ~ 1/sqrt(voices) (wet level is independent of
//    Richness). Density sets the random wait between grains of a voice.
//  * First grain after ~5800 samples + a density-dependent random wait (oracle: 5888..15697).
//  The oracle's random sequence is deterministic but not recoverable; the port models its
//  statistics only, so steady-sine mod_rate/env metrics (random envelope) cannot match.
//  Not modelled: the silent-input Work()==false cut (probes never go silent), 64-sample
//  control blocks. Triage lists Pan/Spread as dead; they are not (per-grain pan, measured).
//  Slider labels use zero-padded [NN] so Faust's label sort keeps Buzz order past 10 params.
import("stdfaust.lib");

raw(s) = floor(s + 0.01);
dry  = raw(hslider("[00]Dry out", 240, 0, 240, 1));
fb   = raw(hslider("[01]Feedback", 0, 0, 99, 1));
full = raw(hslider("[02]Fullness", 0, 0, 100, 1));
rich = raw(hslider("[03]Richness", 12, 4, 24, 1));
dens = raw(hslider("[04]Density", 48, 0, 64, 1));
scat = raw(hslider("[05]Scattering", 48, 0, 64, 1));
fat  = raw(hslider("[06]Fatness", 16, 0, 64, 1));
att  = raw(hslider("[07]Attack", 60, 0, 240, 1));
sus  = raw(hslider("[08]Sustain", 120, 0, 240, 1));
rel  = raw(hslider("[09]Release", 32, 0, 240, 1));
wet  = raw(hslider("[10]Wet out", 240, 0, 240, 1));
pan  = raw(hslider("[11]Pan", 120, 0, 240, 1));
spr  = raw(hslider("[12]Spread", 50, 0, 100, 1));

NV = 24;
NR = 8;          // random streams per voice
MAXD = 65536;

gain(v) = select2(v > 0, 0.0, pow(2.0, (v - 240.0) / 60.0));
seglen(v) = 309.0 * pow(2.0, v / 44.0);
scatrange = 263.0 + 92.0 * pow(2.0, scat / 9.33);
meanwait = 600.0 + 7000.0 * exp(0.0 - dens / 10.1);
densgain = pow(10.0, -0.15 * exp(0.0 - dens / 10.1));
p0 = exp(0.0 - full / 105.0);
p1 = min(1.0 - p0, 0.21);
detune = 0.002 + 0.00006 * fat;
amp = 0.345 * sqrt(12.0 / rich) * densgain;
fbk = 0.4 * fb / 99.0 / (rich * amp);  // per-grain echo 0.14 at N=4; loop gain <= 0.4
first = ba.time < 1;

// Independent LCG per stream (distinct odd increments).
seed(n) = (n + 1) * (n + 1) * 2654435761 + n * 40503;
lcg(n) = (*(1103515245) : +(12345 + seed(n) * (1 - 1'))) ~ _;
uni(i, k) = float(lcg(i * NR + k)) / 4294967296.0 + 0.5;

// One voice: wait W, then a grain of length A+S+R; returns (L, R) for buffer signal b.
voice(i, b) = out
with {
    u(k) = uni(i, k);
    wnew = select2(first, u(0) * 2.0 * meanwait, 5800.0 + u(0) * (5500.0 + 13.6 * (meanwait - 600.0)));
    lg = seglen(att) + seglen(sus) + seglen(rel);
    step(c, per) = cn, pn
    with {
        c1 = c + 1;
        wrap = c1 >= per;
        cn = select2(wrap, c1, 0);
        pn = select2(wrap, per, wnew + lg);
    };
    st = step ~ (_, _);
    c = st : _, !;
    per = st : !, _;
    trig = (c == 0);
    hold(x) = ba.sAndH(trig, x);
    W = hold(per - lg);
    A = hold(seglen(att)); S = hold(seglen(sus)); R = hold(seglen(rel));
    t = c - W;
    env = select2(t < 0, select2(t < A, select2(t < A + S, max(0.0, (A + S + R - t) / R), 1.0), t / A), 0.0);
    k = hold(select2(u(1) < p0, select2(u(1) < p0 + p1, 2.0, 1.0), 0.0));
    dev = hold(select2(fat > 0, 0.0, (u(2) - 0.5) * 2.0 * detune));
    rate = pow(2.0, 0.0 - k) * (1.0 + dev);
    d0 = hold(floor(85.0 + u(3) * scatrange) + 2.0 * k * 0.0);
    d = min(MAXD - 2.0, max(1.0, d0 + (1.0 - rate) * max(t, 0.0)));
    p = hold((1.0 - spr / 100.0) * pan / 240.0 + spr / 100.0 * (0.5 + 0.65 * (u(4) + u(5) - 1.0)));
    active = (i < rich);
    g = amp * env * active;
    s = b : de.fdelay(MAXD, d) : *(g);
    out = s * (1.0 - p), s * p;
};

grains(b) = b <: par(i, NV, voice(i)) :> _, _;

// Buffer = input + feedback * (L+R) of the wet signal.
wetpath(x) = (+(x) : grains) ~ (+ : *(fbk));

process = _, _ : + : *(0.5) <: (*(gain(dry)) <: _, _), (wetpath : *(gain(wet)), *(gain(wet))) :> _, _;
