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
//    The zeros before an onset inside a block are therefore not run through the filter (small loss).
//  * A cutoff at Nyquist (Min 22049, Max 22050) makes the biquad pole-zero cancel: identity.
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

// ---- RBJ low-pass coefficients (b0, b1, a1, a2 already divided by a0) ----
sh(fc)   = sin(ma.PI * fc / ma.SR);
al(fc)   = sin(2 * ma.PI * fc / ma.SR) * damp / 2;
a0(fc)   = 1 + al(fc);
b0(fc)   = sh(fc) * sh(fc) / a0(fc);
a1(fc)   = -2 * (1 - 2 * sh(fc) * sh(fc)) / a0(fc);
a2(fc)   = (1 - al(fc)) / a0(fc);

// ---- block phase: 1 on the first sample of each host block ----
blockstart = bp == 0 with { bp = (\(p).(select2(ma.BS != ma.BS', (p + 1) % ma.BS, 0))) ~ _; };

// ---- gate: has the current block seen a non-zero input yet? ----
awake(x) = (\(a).((x != 0) | (a & (blockstart == 0)))) ~ _;

// ---- the controller + filter, with all state frozen while not awake ----
wah(x, act) = loop ~ si.bus(7) : (!, !, !, !, !, !, !, _)
with {
    loop(x1, x2, y1, y2, fcoff, ss, cnt) =
        hold(x1, x), hold(x2, x1), hold(y1, yo), hold(y2, y1), hold(fcoff, fcn - fc0),
        hold(ss, ssa + (cnta >= 0) * x * x), hold(cnt, cnta + 1), yo * act / sfac
    with {
        fc   = fcoff + fc0;
        upd  = cnt >= period;
        fcn  = select2(upd, fc, glide(fc, target(rmsof(ss, cnt))));
        cnta = select2(upd, cnt, -1);
        ssa  = select2(upd, ss, 0);
        yo   = b0(fcn) * (x + 2 * x1 + x2) - a1(fcn) * y1 - a2(fcn) * y2;
        hold(old, new) = select2(act, old, new);
    };
};

touchwah(x) = wah(x, awake(x));

process = (+ : *(0.5)) : touchwah <: _, _;
