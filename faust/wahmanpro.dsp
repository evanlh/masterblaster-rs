// FSM WahMan Pro (Krzysztof Foltman) -- clean-room Faust port.
//
// Derived only from info.json/presets.json/triage.json and the oracle's
// measurements (per-block least-squares biquad / 4th-order fits on the
// noise-pink development renders; no machine source was read).
//
// Signal path (all measured):
// * Cutoff in "units": u. Filter frequency f = 264 Hz * 2^(u/48)
//   (Cutoff 0/120/240 -> 264/1493.4/8448 Hz exactly).
// * Target T = Cutoff + (LFO mod / 2) * shape(phase). Every 64 samples (grid
//   restarting at each Work chunk = each Faust block, since masterblaster
//   chunks ticks into 256-sample blocks) u moves toward T by at most
//   10 * exp(-0.0375 * Inertia) units (linear slew, starts at u = 0).
//   Coefficients are recomputed only at those block starts.
// * LFO phase advances per sample at 0.05 Hz * exp(rate * 6.29139/240)
//   (0.05 .. 27 Hz, not tempo synced); Set phase p sets phase = p/128.
// * Shapes (x = 2 pi phase): 0 sin x; 1 saw 4ph-3; 2 saw 3-4ph; 3 square;
//   4 triangle; 5 sin(x + pi/4 sin x); 6 sin(x + pi/6 sin 2x);
//   7 sin(2x + pi cos 3x); 8 0.5 sin 2x + 0.5 cos 3x.
// * Filters (DF1 biquads; output state clipped at +-300000 in Buzz units,
//   seen when f passes Nyquist and the peaking filter goes unstable):
//   0 LP: bilinear 1/(A s^2 + B s + 1), A = 1-d, B = 2d(1-d),
//     1/d = 1.163 + 0.0865 r + 8.65e-4 r^2 (fit to r = 0, 92, 120, 240)
//   1 peak: p = sin w/20, gain G = 1.01 + r/8 (a0 = 1+p, b0 = 1+G p)
//   2 peak: p = sin w/(2(1+r/6)), G = 32
//   3 LP4: two LP sections with 1/d = 1.439 + 0.04182 r, at K and K*0.7
//   4 peak x2: Q' = 1+r/12, G = 6+r/30, at f and f/(1+r/240)
// * Set phase default is 255 (no-value) in Buzz, outside 0..127; the slider
//   init is 0 (only a change of the slider resets the phase).
import("stdfaust.lib");

cutoff = hslider("[0]Cutoff", 120, 0, 240, 1) : rint;
reso   = hslider("[1]Resonance", 120, 0, 240, 1) : rint;
ftype  = hslider("[2]Filter", 1, 0, 4, 1) : rint;
rate   = hslider("[3]LFO rate", 30, 0, 240, 1) : rint;
depth  = hslider("[4]LFO mod", 60, 0, 240, 1) : rint;
shape  = hslider("[5]LFO Shape", 1, 0, 8, 1) : rint;
inert  = hslider("[6]Inertia", 10, 0, 60, 1) : rint;
setph  = hslider("[7]Set phase", 0, 0, 127, 1) : rint;

SR = ma.SR;
TWOPI = 2.0 * ma.PI;
CLIP = 300000.0 / 32768.0;

// ---- block grid -------------------------------------------------------
first = 1 - (1 : mem);
remaining = (\(r).(select2(r <= 0, r - 1, ma.BS - 1))) ~ _;
newblock = (remaining : mem) <= 0;
blockpos = (\(p).(select2(newblock, p + 1, 0))) ~ _;
gridstart = (blockpos % 64) == 0;

// ---- LFO ----------------------------------------------------------------
lfohz = 0.05 * exp(rate * 6.29139 / 240.0);
frac(v) = v - floor(v);
reset = first | (setph != (setph : mem));
phase = (\(ph).(select2(reset, frac(ph + lfohz / SR), setph / 128.0))) ~ _;

shapev(ph) = ba.selectn(9, shape,
    sin(x),
    4.0 * ph - 3.0,
    3.0 - 4.0 * ph,
    select2(ph < 0.5, -1.0, 1.0),
    select2(ph < 0.5, 3.0 - 4.0 * ph, 4.0 * ph - 1.0),
    sin(x + ma.PI / 4.0 * sin(x)),
    sin(x + ma.PI / 6.0 * sin(2.0 * x)),
    sin(2.0 * x + ma.PI * cos(3.0 * x)),
    0.5 * sin(2.0 * x) + 0.5 * cos(3.0 * x))
with { x = TWOPI * ph; };

// ---- cutoff slew ----------------------------------------------------------
stepmax = 10.0 * exp(-0.0375 * inert);
target = cutoff + depth * 0.5 * shapev(phase);
slew(t) = (\(u).(select2(gridstart, u, u + max(-stepmax, min(stepmax, t - u))))) ~ _;
units = slew(target);
freq = 264.0 * pow(2.0, units / 48.0);

// ---- biquad sections (coefficient tuples b0,b1,b2,a1,a2) -----------------
peak(w, q, g) = (1.0 + g * p) / a0, -2.0 * cos(w) / a0, (1.0 - g * p) / a0,
                -2.0 * cos(w) / a0, (1.0 - p) / a0
with { p = sin(w) / (2.0 * q); a0 = 1.0 + p; };

lowpass(k, d) = k2 / dd, 2.0 * k2 / dd, k2 / dd,
                2.0 * (k2 - a) / dd, (a - b * k + k2) / dd
with { a = 1.0 - d; b = 2.0 * d * (1.0 - d); k2 = k * k; dd = a + b * k + k2; };

unity = 1.0, 0.0, 0.0, 0.0, 0.0;

w0 = TWOPI * freq / SR;
k0 = tan(0.5 * w0);
d0 = 1.0 / (1.163 + 0.0865 * reso + 0.000865 * reso * reso);
d3 = 1.0 / (1.439 + 0.04182 * reso);
q4 = 1.0 + reso / 12.0;
g4 = 6.0 + reso / 30.0;

el(i, tup) = tup : ba.selector(i, 5);
pick(i, t0, t1, t2, t3, t4) = ba.selectn(5, ftype, el(i, t0), el(i, t1), el(i, t2),
                                         el(i, t3), el(i, t4));

sec1(i) = pick(i, lowpass(k0, d0), peak(w0, 10.0, 1.01 + reso / 8.0),
               peak(w0, 1.0 + reso / 6.0, 32.0), lowpass(k0, d3), peak(w0, q4, g4));
sec2(i) = pick(i, unity, unity, unity, lowpass(k0 * 0.7, d3),
               peak(w0 / (1.0 + reso / 240.0), q4, g4));

clip(v) = max(-CLIP, min(CLIP, v));
df1(b0, b1, b2, a1, a2, x) = (\(y).(clip(ff - a1 * y - a2 * (y : mem)))) ~ _
with { ff = b0 * x + b1 * (x : mem) + b2 * (x : mem : mem); };

section(c) = df1(c(0), c(1), c(2), c(3), c(4));
wah = section(sec1) : section(sec2);

process = + : *(0.5) : wah <: _, _;
