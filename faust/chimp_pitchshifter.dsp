// Chimp's PitchShifter (Dave Hooper) -- clean-room Faust port, derived only from the
// oracle's measurements (no machine source was read).
//
// What the oracle showed (noise-white raw output matched sample-exactly, >255 dev configs):
//  - One read head into a delay line of the live input, NO crossfade and NO interpolation:
//    y_wet[n] = x[n - d[n]] with an integer d.  d[n] = floor(acc[n]) where acc is a float
//    that moves by (1 - ratio) per sample, ratio = 2^(+-Shift/1200) (Direction 1 = up).
//    So shifting down grows the delay (sample repetition: x0,x0,x1,x1 at -1 octave) and
//    shifting up shrinks it.  acc wraps inside a window of 1024 samples: >= 1024 -> -1024
//    and < 0 -> +1024.  The jump is a hard splice (period 1024/|1-ratio|, e.g. 2048 samples
//    at -1 octave, 341.3 at +2 octaves).  Initial acc = 0.9999 (fits "first cycle ceil-like,
//    later cycles floor-like" at every tested ratio).  The oracle matches float64 accumulation exactly; Faust is
//    float32 so acc is carried as integer part + fraction. Remaining error: float32 s=1-ratio
//    makes the splice slip by one sample in ~0.1% of samples for non-dyadic shifts (e.g. 600,
//    1800 cents). A double-float (hi/lo pair) version removed that in numpy but made libfaust
//    compile for minutes, so it was dropped.
//  - acc is ONE state shared by both directions (Direction toggles keep the delay continuous).
//    Parameter changes apply at once (acc continues, only the step changes).
//  - Gains: dry = (raw-64)/64, wet = (raw-64)/64 (describe text: 40h=0%, 80h=100%).
//    Special cases measured: Dry+Wet == 128 raw -> exact silence; Wet == 64 or Shift == 0
//    -> input passes untouched (no dry/wet mixing).  While muted/bypassed the read head is
//    stopped (acc grows by 1/sample, the write side keeps running): seen as a delay of
//    ~261+5250 right after a bypassed tick, partly (first block) -- the block-level wrap of
//    that large delay is not modelled.
//  - work_returns_false: the output is cut (0) for a 256-sample block whose input is all
//    silent, blocks on a grid restarting every tick (5250 samples at 126 BPM / 4 TPB;
//    measured on the ess tail: output ends exactly at the end of the block holding the last
//    non-zero input).  Emulated causally (output 0 while the block so far is silent), so a
//    block that starts silent and gets signal mid-block loses its head.  The tick length
//    5250 is hard-coded (the oracle tempo).
//  - Not modelled: state not advancing during cut blocks (the oracle never resumes after a cut).
import("stdfaust.lib");

shift = hslider("[0]Shift", 1200, 0, 2400, 1);
dirp = hslider("[1]Direction", 0, 0, 1, 1);
dryp = hslider("[2]Dry out", 128, 0, 128, 1);
wetp = hslider("[3]Wet out", 128, 0, 128, 1);

TICK = 5250;
BLOCK = 256;
WINDOW = 1024;
ACC0 = 0.9999;

raw(s) = floor(s + 0.5);
cents = raw(shift);
up = raw(dirp) > 0.5;
dry = raw(dryp);
wet = raw(wetp);

ratio = pow(2.0, select2(up, 0 - cents, cents) / 1200.0);
mute = (dry + wet) == 128;
bypass = (wet == 64) | (cents == 0);
step = select2(mute | bypass, 1.0 - ratio, 1.0);
dryGain = (dry - 64) / 64.0;
wetGain = (wet - 64) / 64.0;

// head position acc = I + f; returns the wrapped integer part (the delay in samples)
sweep = (advance ~ (_, _)) : (!, !, _)
with {
    advance(pI, pf, dl) = nI, nf, wI
    with {
        wI = pI - WINDOW * (pI >= WINDOW) + WINDOW * (pI < 0);
        dI = floor(dl);
        fs = pf + (dl - dI) + ACC0 * (ba.time == 0);
        carry = fs >= 1.0;
        nf = fs - carry;
        nI = wI + dI + carry;
    };
};

// 1 while the current 256-block (grid restarts each tick) has had any non-zero input
blockActive(l, r) = loop ~ _
with {
    blockStart = ((ba.time % TICK) % BLOCK) == 0;
    loop(p) = max(p * (1 - blockStart), (abs(l) + abs(r)) > 0);
};

voice(d, active, x) = active * select2(mute, select2(bypass, dryGain * x + wetGain * wetx, x), 0.0)
with {
    wetx = de.delay(WINDOW, d, x);
};

process(l, r) = voice(d, a, l), voice(d, a, r)
with {
    d = step : sweep;
    a = blockActive(l, r);
};
