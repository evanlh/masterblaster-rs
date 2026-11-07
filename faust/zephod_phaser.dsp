// Zephod Phaser — clean-room port (no machine source read).
//
// Derived only from info.json and the buzz-machines-oracles measurements
// (noise-white / impulse / ess dev renders, per-block and per-sample fits).
// Despite its name the machine is a feedback comb on a long delay line:
//
//   y[n]   = x[n] + g[n] * buf[n - D]          (output, both channels)
//   buf[n] = x[n] + f[n] * y[n]                (delay line holds input + f * output)
//
//   D = max(1, Depth - 1) samples              (cross-correlation peak: Depth 751 -> 750,
//                                               3000 -> 2999, 1 -> 1)
//   g = follow(Amount / 255)                   (static fits: 65 -> 0.2549, 128 -> 0.50196)
//   f = clamp(follow(Feedback / 10000) + lfo, 0, 1)
//   lfo = LFOamplitide / 10000 * sin(6.28 * p), with the same attack-only follower
//         applied only while that product is positive and rising
//   p  += LFOfrequency * 1e-7 per sample, wrapped to [0, 1); right channel p + 0.5
//
// "follow" is a one-pole attack (0.98 old + 0.02 new) with instant release:
// Amount / Feedback jumps ramp up with ratio 0.98/sample and drop in one sample.
// The LFO lags by exactly ~50 samples only in its first quarter (positive, rising)
// and is exact elsewhere, which is what that follower produces. The phase factor
// 6.28 (not 2*pi) is measured: the LFO drifts by 0.0032 rad per cycle and jumps
// back at each wrap; 1e-7 cycles/sample per LFOfrequency unit is fitted (9.9997e-8).
//
// LFOshift is dead in the measurements: the right channel is always the left
// LFO shifted by half a cycle (shift 0, 325, 500, 1000 all measure pi). It is
// kept as a slider for parameter fidelity.
//
// Defaults: Buzz reports default 0 for Depth, LFOfrequency, LFOamplitide and
// Amount, outside their 1.. ranges (0 is their no_value); Faust rejects an init
// outside the range, so those sliders start at their minimum (1), which is what
// the oracle's defaults configuration also plays.
//
// Depth changes: the read uses the previous sample's delay and the line is
// cleared (jump 1 -> 3000 reads zeros for 2999 samples, 3000 -> 1 reads the
// sample just written), emulated by ignoring reads older than the last change.
//
// Silent input: work_returns_false. Buzz skips 256-sample blocks (grid restarting
// every tick; 5250 samples at the oracle's 126 BPM / 4 TPB, since masterblaster
// gives Faust no tempo) whose input is all zero: output is zero and no state
// advances (the Amount ramp in the impulse probe has advanced exactly from the
// start of the impulse's block). Emulated causally: output is muted while the
// block has been silent so far, and all state rolls back to the block start
// snapshot if the whole block was silent.
//
// Known gaps: the first ~300 samples after start differ slightly (follower start
// conditions); Feedback ~ 1 with Amount ~ 1 is a marginally stable loop that
// diverges chaotically from any float-rounding difference.
import("stdfaust.lib");

depth  = hslider("[0]Depth", 1, 1, 3000, 1) : rint;
fbk    = hslider("[1]Feedback", 0, 0, 10000, 1) : rint;
lfrq   = hslider("[2]LFOfrequency", 1, 1, 3000, 1) : rint;
lamp   = hslider("[3]LFOamplitide", 1, 1, 10000, 1) : rint;
lshift = hslider("[4]LFOshift", 0, 0, 1000, 1) : rint;
amount = hslider("[5]Amount", 1, 1, 255, 1) : rint;

TICK = 5250;
BLK = 256;
BUFN = 4096;

// --- block grid and silence ---------------------------------------------------
pos = ((+(1) : %(TICK)) ~ _) + TICK - 1 : %(TICK);
bs = (pos % BLK) == 0;
silentSoFar(z) = (\(sp).(select2(bs, sp * z, z))) ~ _;

// --- attack-only follower -----------------------------------------------------
follow(cur, tgt) = select2(tgt > cur, tgt, 0.98 * cur + 0.02 * tgt);

// State with block rollback: returns (updated value, value entering this sample).
rb(restore, U) = loop ~ (_, _) : (_, !, _)
with {
    loop(xp, sp) = U(xi), select2(bs, sp, xi), xi
    with { xi = select2(restore, xp, sp); };
};

wrap1(v) = v - (v >= 1.0);

phaser(xl, xr) = outl, outr
with {
    x = (xl + xr) * 0.5;
    sil = silentSoFar(x == 0);
    restore = bs * sil';

    g = rb(restore, \(c).(follow(c, amount / 255.0))) : (_, !);
    fs = rb(restore, \(c).(follow(c, fbk / 10000.0))) : (_, !);
    p = rb(restore, \(c).(wrap1(c + lfrq * 1e-7))) : (!, _);
    widx = rb(restore, \(c).(c + 1)) : (!, _) : int;

    d = max(1, depth - 1) : int;
    dr = max(1, d') : int;
    clr = (\(c).(select2(d != d', c, widx))) ~ _;

    chan(ph) = y
    with {
        v = sin(6.28 * ph);
        a = lamp / 10000.0 * v;
        ls = rb(restore, \(c).(select2((a > 0) * (a > c), a, follow(c, a)))) : (_, !);
        f = min(1.0, max(0.0, fs + ls));
        ridx = widx - dr;
        valid(r) = select2(ridx >= clr, 0.0, r);
        tap(sprev) = rwtable(BUFN, 0.0, widx' & (BUFN - 1), sprev, ridx & (BUFN - 1)) : valid;
        step(sprev) = x + f * yv, yv
        with { yv = x + g * tap(sprev); };
        y = step ~ _ : (!, _);
    };

    keep = (lshift > 5000) * x;  // never true: keeps the dead slider in the UI
    outl = select2(sil, chan(p), 0.0) + keep;
    outr = select2(sil, chan(wrap1(p + 0.5)), 0.0);
};

process = phaser;
