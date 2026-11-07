// Chun-Yu's TouchWah -- envelope-controlled resonant low-pass ("touch wah").
// Clean-room port derived only from the oracle's measurements (no machine source).
//
// What the oracle shows (each point checked sample-exactly on development raw files):
//  * Audio path: RBJ-cookbook 2-pole low-pass (bilinear, direct form I), Q = 1 / (1.51 - Resonance/100)
//    (fits Resonance 10..141 -> Q 0.709 .. 10.0 to 3 digits). Initial cutoff is 11025 Hz.
//  * Every N = round(44100 / Changes/Sec) samples the controller looks at the RMS of the last N input
//    samples (the sample on which the update fires is not counted, so the period is N + 1 after the
//    first update). Target cutoff = lo + |max - min| * rms, lo = min(Min Cutoff, Max Cutoff): the two
//    cutoffs are interchangeable. The cutoff glides toward the target: half way when falling, three
//    quarters of the way when rising (fc += k * (target - fc)).
//  * In->SinCurve? maps the RMS through a sine-like curve first. The oracle's curve is
//    x - x^3/4 + x^5/40 with x = rms * pi / 2 (fitted to 0 < rms < 0.72; close to but not equal to
//    sin(x), and the original may be a table).
//  * ScaleFactor divides only the output (the RMS is measured before it).
//  * Silent input: the machine returns false from Work on blocks (256 samples, restarting each
//    tick) whose input is entirely zero; output is 0 and all state (filter, window counters, cutoff)
//    freezes. Emulated causally: while the current block has been silent so far, output 0 and hold state.
//    At the first non-zero sample of a block the window counter and the filter state are caught up
//    over the leading zeros (counter exactly, filter via its closed-form free response); only the
//    machine's ringing output during those leading zeros is lost.
//  * Remaining differences: (1) Min 22049 / Max 22050 puts the poles on z = -1 next to a double zero;
//    the oracle (double precision state, sample-exact to ~1e-7 in the first blocks) diverges from
//    single-precision Faust within a few thousand samples, so those corner jobs score ~0. (2) cutoffs
//    of a few Hz (Min 1 / Max 2) give outputs about -70 dB below the input and are not matched.
//    (3) The sin curve is an empirical fit, accurate to ~1e-3 in the target amplitude.
// Raw parameter values are recovered by rounding (masterblaster scales raw -> 0..65535 -> slider by
// truncation). Mono machine: both Faust inputs are summed and halved, the result goes to both outputs.
import("stdfaust.lib");

res    = int(hslider("[0]Resonance", 129, 10, 141, 1) + 0.5);
minc   = int(hslider("[1]Min Cutoff (Hz)", 256, 1, 22049, 1) + 0.5);
maxc   = int(hslider("[2]Max Cutoff (Hz)", 11025, 2, 22050, 1) + 0.5);
cps    = int(hslider("[3]Changes/Sec", 32, 1, 128, 1) + 0.5);
sinc   = hslider("[4]In->SinCurve?", 0, 0, 1, 1) > 0.5;
sfac   = int(hslider("[5]ScaleFactor", 1, 1, 16, 1) + 0.5);

// ---- controller parameters (floats) ----
fc0    = 11025.0;  // initial cutoff; the cutoff state is stored as an offset from it (Faust state starts at 0)
period = int(ma.SR / cps + 0.5);
lo     = min(minc, maxc);
span   = abs(maxc - minc);
damp   = 1.51 - res / 100.0;

// ---- amplitude -> cutoff ----
curve(r) = select2(sinc, r, xx - xx * xx * xx / 4 + pow(xx, 5) / 40) with { xx = r * ma.PI / 2; };
target(rms) = lo + span * curve(rms);
glide(fc, t) = fc + select2(t > fc, 0.5, 0.75) * (t - fc);
rmsof(ss, cnt) = sqrt(ss / max(cnt, 1));

// ---- RBJ low-pass coefficients (divided by a0) ----
sh(fc)   = sin(ma.PI * fc / ma.SR);
al(fc)   = sin(2 * ma.PI * fc / ma.SR) * damp / 2;
a0(fc)   = 1 + al(fc);
b0(fc)   = sh(fc) * sh(fc) / a0(fc);
// a1 + 2 and a2 - 1 are formed directly: the poles sit near z = +-1 at extreme cutoffs, where
// a1 = -2 and a2 = 1 would be indistinguishable in single precision.
da1(fc)  = (2 * al(fc) + 4 * sh(fc) * sh(fc)) / a0(fc);
da2(fc)  = -2 * al(fc) / a0(fc);

// ---- free (zero-input) response of the biquad, for catching up over leading zeros ----
// Poles: radius sqrt(a2) = sqrt((1-al)/(1+al)), angle atan2(sin(w) sqrt(1-damp^2/4), cos(w)).
// With U(k) = r^k sin((k+1) theta) / sin(theta):  y[n+p] = U(p) y[n] - a2 U(p-1) y[n-1].
wc(fc)      = 2 * ma.PI * fc / ma.SR;
theta(fc)   = atan2(sin(wc(fc)) * sqrt(1 - damp * damp / 4), cos(wc(fc)));
rad2(fc)    = (1 - al(fc)) / (1 + al(fc));
ufun(fc, k) = pow(sqrt(rad2(fc)), k) * sin((k + 1) * theta(fc)) / sin(theta(fc));
freey1(fc, p, ya, yb) = ufun(fc, p) * ya - rad2(fc) * ufun(fc, p - 1) * yb;
freey2(fc, p, ya, yb) = ufun(fc, p - 1) * ya - rad2(fc) * ufun(fc, p - 2) * yb;

// ---- position inside the host block (0 on its first sample) ----
blockpos = (\(p).(select2(ma.BS != ma.BS', (p + 1) % ma.BS, 0))) ~ _;

// ---- gate: has the current block seen a non-zero input yet? ----
awake(x) = (\(a).((x != 0) | (a & (blockpos != 0)))) ~ _;

// ---- the controller + filter, with all state frozen while not awake ----
// At the first non-zero sample of a block the window counter catches up with the leading zeros the
// machine would have processed (they carry no energy); an update falling inside them is applied.
wah(x, act) = loop ~ si.bus(7) : (!, !, !, !, !, !, !, _)
with {
    onset = act * (1 - act');
    loop(x1, x2, y1, y2, fcoff, ss, cnt) =
        hold(x1, x), hold(x2, x1e), hold(y1, yo), hold(y2, y1e), hold(fcoff, fcn - fc0),
        hold(ss, ssa + (cnta >= 0) * x * x), hold(cnt, cnta + 1), yo * act / sfac
    with {
        fc   = fcoff + fc0;
        c0   = cnt + onset * blockpos;
        cut  = c0 > period;                      // an update fired inside the leading zeros
        fc1  = select2(cut, fc, glide(fc, target(rmsof(ss, period))));
        ss1  = select2(cut, ss, 0);
        cnt1 = select2(cut, c0, c0 - period - 1);
        upd  = cnt1 >= period;
        fcn  = select2(upd, fc1, glide(fc1, target(rmsof(ss1, cnt1))));
        cnta = select2(upd, cnt1, -1);
        ssa  = select2(upd, ss1, 0);
        y1e  = select2(onset, y1, freey1(fc, blockpos, y1, y2));   // (y1, y2) advanced over the
        y2e  = select2(onset, y2, freey2(fc, blockpos, y1, y2));   // leading zeros of the block
        x1e  = x1 * (1 - onset);
        x2e  = x2 * (1 - onset);
        yo   = 2 * y1e - y2e + b0(fcn) * (x + 2 * x1e + x2e) - da1(fcn) * y1e - da2(fcn) * y2e;
        hold(old, new) = select2(act, old, new);
    };
};

touchwah(x) = wah(x, awake(x));

process = (+ : *(0.5)) : touchwah <: _, _;
