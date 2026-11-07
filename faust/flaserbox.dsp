// Rymix FlaserBox — clean-room Faust port (flanger / tape flanger / phaser, 13 routings).
//
// Derived only from info.json/describe.json/presets.json/triage.json and oracle measurements
// (per-block delay/allpass fits on noise-pink renders against the dry input). No machine
// source was read.
//
// Measured facts (126 BPM, 4 TPB, 44.1 kHz => 5250 samples/tick):
//  * Output = Gain/127 * (sin(pi/2*WD/254)*dry + cos(pi/2*WD/254)*wet)  (exact LS fit).
//  * Level params: (v-127)/127 scale the processor outputs.
//  * Flanger: delay (samples) = 1 + floor(min*k) + floor(range*k)*(1-lfo)/2,
//    k = sr*0.0390625 ms (describe: 0x80 = 5 ms). Linear interpolation, write = in + fb*out,
//    fb = (v-512)/512 (no limiter: +-100% stays bounded as in the oracle).
//  * LFO: period = Rate/4 ticks, phase starts at 0, Rate 0 freezes. Manual = +m/1024 cycle,
//    Phase Sep = +p/254 cycle on the right channel. Step = sample&hold of the LFO every
//    Step/4 ticks (held from t=0). Shapes: sin, tri (flanger +1 / phaser -1 at phase 0),
//    square (+1 first half), saw up (-1..1), saw down; Noise is modelled statistically (random S&H every 8 samples); its sequence is not
//    recoverable (those renders are nondeterministic).
//  * Tape flanger = flanger + x(t - floor(range*k)/2) - x; the tape term is not scaled by the
//    levels and bypasses the HiPass/LoPass (fits on presets Wormhole / Spaceship Engine).
//  * Phaser: N first-order allpasses y = c v - v' + c y' (odd N with c=1 is +1, measured),
//    c = (1 - tan(pi f/sr))/(1 + tan(pi f/sr)),
//    f = floor(range*k)*(1+lfo)/2 (Min has no audible effect), feedback out->in (v-512)/512.
//  * Routings (s = cos(pi/2*WD/254): the inner stage of a chain carries the wet gain again,
//    fitted at WD 64/127/191 in modes 3 and 9): 3/5 s*P(F)/s*F(P), 4/6 same + tape tap, 7 P(x+sF),
//    8 P(tap+sF), 9 F(s(x+P)), 10 9+tape, 11 F+P, 12 T+P; levels: 7/8 lP*P(.. + s*lF*F),
//    others scale by lF*lP (fits on mode-7/9 level corners and presets). In modes 9/10 both
//    flangers are fed the LEFT phaser (stereo-matrix: right output while only left plays).
//  * Step S&H period is Step/4 ticks + 1 sample (edges at 42001, 84002, 126003, 168004).
//  * Noise LFO values are uniform in [-1,1] (fit per Step-held segment on preset Random5).
//  * HiPass/LoPass: 2nd-order Butterworth on the wet path, f = 20 + 19980*(v/32768)^2.
//  * Link bits copy flanger values to the phaser (1 min,2 range,4 fb,8 rate,16 step,32 man,
//    64 phase, 128 all + level) — from the parameter text; measured: a linked Manual freezes
//    the phaser LFO at its manual position.
//  * Inertia only smooths parameter changes (not exercised by static renders): ignored.
//  * Faust trims the leading spaces of labels such as "  Minimum"; order/ranges/defaults match.
//  * Tempo is not visible to Faust: the tick length assumes 126 BPM / 4 TPB.
import("stdfaust.lib");

gain   = hslider("[00]Gain", 127, 0, 254, 1);
mode   = hslider("[01]Mode", 0, 0, 12, 1);
link   = hslider("[02]Link", 0, 0, 128, 1);
flvl   = hslider("[03]Flanger Lvl ----", 254, 0, 254, 1);
fmin   = hslider("[04]  Minimum", 13, 0, 1024, 1);
frng   = hslider("[05]  Range", 128, 0, 1024, 1);
ffb    = hslider("[06]  Feedback", 512, 0, 1024, 1);
frate  = hslider("[07]  Rate", 128, 0, 2048, 1);
fstep  = hslider("[08]  Step", 0, 0, 128, 1);
fman   = hslider("[09]  Manual LFO*", 0, 0, 1024, 1);
fsep   = hslider("[10]  Phase Sep", 0, 0, 254, 1);
plvl0  = hslider("[11]Phaser Lvl ----", 254, 0, 254, 1);
stages = hslider("[12]  Stages", 12, 0, 48, 1);
pmin0  = hslider("[13]  1.Min", 13, 0, 1024, 1);
prng0  = hslider("[14]  2.Range", 128, 0, 1024, 1);
pfb0   = hslider("[15]  4.Feedback", 512, 0, 1024, 1);
prate0 = hslider("[16]  8.Rate", 128, 0, 2048, 1);
pstep0 = hslider("[17]  16.Step", 0, 0, 128, 1);
pman0  = hslider("[18]  32.Manual*", 0, 0, 1024, 1);
psep0  = hslider("[19]  64.Phase", 0, 0, 254, 1);
ltype  = hslider("[20]LFO Type F:P", 0, 0, 35, 1);
hip    = hslider("[21]HiPass*", 0, 0, 32768, 1);
lop    = hslider("[22]LoPass*", 32768, 0, 32768, 1);
inertia = hslider("[23]Inertia", 0, 0, 2048, 1);
wetdry = hslider("[24]Wet/Dry", 127, 0, 254, 1);

// ---- parameter conversion ----
r = rint;
tick = ma.SR * 60.0 / (126.0 * 4.0);
kms = ma.SR * 0.0390625 * 0.001;
level(v) = (r(v) - 127.0) / 127.0;
fbk(v) = (r(v) - 512.0) / 512.0;
bit(b) = ((int(r(link)) & b) > 0) | (r(link) >= 128);
linked(b, f, p) = select2(bit(b), p, f);

prng  = linked(2, frng, prng0);
pfb   = linked(4, ffb, pfb0);
prate = select2(bit(32), linked(8, frate, prate0), 0);   // linked Manual stops the phaser LFO (measured)
pstep = linked(16, fstep, pstep0);
pman  = linked(32, fman, pman0);
psep  = linked(64, fsep, psep0);
plvl  = select2(r(link) >= 128, plvl0, flvl);
keep = select2((inertia + pmin0) < 0, 1.0, 0.0);   // Inertia, 1.Min: no static effect

// ---- LFO ----
// phase in cycles from an exact integer counter: one period = Rate quarter-ticks, and a
// quarter-tick is QT/2 samples (QT = 2625 at 44.1 kHz), so 2*samples counts modulo Rate*QT.
// Rate 0 freezes. The first sample sees phase 0.
QT = int(rint(ma.SR * 30.0 / 504.0));
icount(len) = (+(2) : wrapi) ~ _ : mem with { wrapi(c) = select2(c >= len, c, c - len); };
phase(rate) = select2(len > 0, 0, float(icount(len)) / float(max(1, len)))
  with { len = int(r(rate)) * QT + int(rint(r(rate) * r(rate) / 1024.0)); };
  // The oracle's LFO runs slightly slow: square edges at 84005/168009 instead of 84000/168000
  // for Rate 128 (sine fits: period 168008), no measurable drift at Rate 8, and more at
  // Rate 1024. Modelled as Rate^2/2048 extra samples per period (likely float phase
  // accumulation in the original). Rate 0 holds phase 0.

shape(t, p, tsign) = ba.selectn(6, t, sine, tri, sqr, sawu, sawd, nz) with {
  sine = sin(2.0 * ma.PI * p);
  tri  = tsign * (4.0 * abs(p - 0.5) - 1.0);   // flanger starts at +1, phaser at -1 (measured)
  sqr  = select2(p < 0.5, -1.0, 1.0);
  sawu = 2.0 * p - 1.0;
  sawd = 1.0 - 2.0 * p;
  // Noise: uniform random value held for 8 samples (the 8-sample hold matches the oracle's
  // third-octave spectrum at Rate 128; L and R share one sequence, as in the oracle).
  nz   = no.noise : ba.sAndH(ba.time % 8 == 0);
};

// sample & hold every step/4 ticks (0 = off), held from t=0
// sample & hold every Step quarter-ticks (0 = off), first sample at t=0
stepped(st, v) = select2(r(st) > 0, v, ba.sAndH(trig, v)) with {
  len = int(r(st)) * QT + 2;   // holds last Step/4 ticks + 1 sample (edges at 42001, 84002, ...)
  cnt = (+(2) : wrapc) ~ _;
  wrapc(c) = select2(c >= len, c, c - len);
  trig = (cnt' < 2);
};

wrap1(x) = ma.frac(x);
lfo(t, tsign, rate, st, man, sep, ch) = stepped(st, shape(t, wrap1(phase(rate) + r(man) / 1024.0 + ch * r(sep) / 254.0), tsign));

// ---- flanger ----
MAXD = 8192;
tapf(s, d) = s @ i * (1.0 - fr) + s @ (i + 1) * fr with {
  dc = max(0.0, min(MAXD - 2, d));
  i = int(dc);
  fr = dc - floor(dc);
};
flange(x, d, fb) = tapf(w, d) with {
  w = (fbin : +(x)) ~ _;
  fbin(wp) = fb * tapf(wp, d - 1.0);
};
fdelay(v) = 1.0 + floor(r(fmin) * kms) + floor(r(frng) * kms) * (1.0 - v) * 0.5;
tapfix(x) = x @ int(max(0.0, min(MAXD - 2, floor(floor(r(frng) * kms) * 0.5))));

// ---- phaser ----
ap(c) = _ <: (*(c)), (mem : *(-1.0)) :> _ : (+ ~ *(c));   // y = c v - v' + c y'
chain(c) = seq(i, 48, outs(i)) : sel with {
  outs(i) = par(j, i, _), (_ <: _, ap(c));
  sel = par(i, 48, _), _ : ba.selectn(49, r(stages));
};
coef(v) = (1.0 - a) / (1.0 + a) with {
  a = tan(ma.PI * min(f, 0.45 * ma.SR) / ma.SR); f = floor(r(prng) * kms) * (1.0 + v) * 0.5; };
phaser(x, c, fb) = (+(x) : chain(c)) ~ *(fb);

// ---- channel ----
m = r(mode);
wdang = 0.5 * ma.PI * r(wetdry) / 254.0;
cw = cos(wdang);   // chained stages apply the wet gain once more (measured cos^2 law)
lf = level(flvl);
lp = level(plvl);
vf(ch) = lfo(int(r(ltype)) % 6, 1.0, frate, fstep, fman, fsep, ch);
vp(ch) = lfo(int(r(ltype) / 6), -1.0, prate, pstep, pman, psep, ch);
fxOf(ch, x) = flange(x, fdelay(vf(ch)), fbk(ffb));

// phaser output of one channel (its input depends on the routing)
phaserOf(ch, x) = phaser(pin, coef(vp(ch)), fbk(pfb)) with {
  fx = fxOf(ch, x);
  fix = tapfix(x);
  pin = ba.selectn(13, m, x, x, x, fx, fx, x, x, x + cw * lf * fx, fix + cw * lf * fx, x, x, x, x);
};

// pa: this channel's phaser; pf: the phaser feeding the [P]->F flanger. In modes 9/10 the
// oracle feeds the LEFT phaser into both flangers (right output leaks left input; measured
// on stereo-matrix: right output is non-zero while only the left input plays).
finish(ch, x, pa, pf) = out with {
  fx = fxOf(ch, x);
  tape = tapfix(x) - x;
  fin = ba.selectn(13, m, x, x, x, x, x, pa, pa, x, x, cw * (x + pf), cw * (x + pf), x, x);
  fb2 = flange(fin, fdelay(vf(ch)), fbk(ffb));
  core = ba.selectn(13, m,
          lf * fx, lf * fx, lp * pa,
          lf * lp * cw * pa, lf * lp * cw * pa,
          lf * lp * cw * fb2, lf * lp * cw * fb2,
          lp * pa, lp * pa,
          lf * lp * fb2, lf * lp * fb2,
          lf * fx + lp * pa, lf * fx + lp * pa);
  isTape = (m == 1) | (m == 4) | (m == 6) | (m == 10) | (m == 12);
  wet = (core : hpf : lpf) + isTape * tape;
  hz(v) = 20.0 + 19980.0 * (r(v) / 32768.0) ^ 2;
  hpf = _ <: select2(r(hip) > 0, _, fi.highpass(2, hz(hip)));
  lpf = _ <: select2(r(lop) < 32768, _, fi.lowpass(2, hz(lop)));
  out = (r(gain) / 127.0) * (sin(wdang) * x + cw * wet) * keep;
};

process(xl, xr) = finish(0, xl, pl, pl), finish(1, xr, pr, pl) with {
  pl = phaserOf(0, xl);
  pr = phaserOf(1, xr);
};
