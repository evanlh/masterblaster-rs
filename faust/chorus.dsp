// Jeskola Chorus — clean-room port (no machine source read).
//
// Derived only from info.json / describe.json and the buzz-machines-oracles measurements
// (noise-pink, impulse and burst-then-silence development renders; per-block cross-correlation,
// least-squares tap fits and a sample-accurate numpy model that reproduces every development
// job to float rounding). Findings:
//
// * Levels: dry, wet and feedback are (v - 64) / 64 (describe.json: 0 = -100%, 64 = 0%, 128 = +100%).
// * Delay line: 4096-sample circular buffer; the buffer input is x + feedback * wet; taps are read
//   before the current sample is written (a tap at delay 0 or less reads the 4096-old cell).
// * Delay position: 16.16 fixed point, ping-ponging between lo = minDelay*44.1 samples + 1 and
//   hi = lo + depth/127 * 441 samples (both truncated to 1/65536). Each sample it moves by
//   step = floor(R * (freq/100) / 44100) (fixed point, truncated: this gives the measured
//   0.9992 rate factor at freq 100 / depth 64) and reflects off a bound once per sample. The
//   position starts at 0, so it bounces into range at start-up: with freq 0 the delay sits at
//   fold(0) (explains the parameter-dependent constant delays), with depth 0 it alternates
//   between 0 and 2*lo every sample.
// * Interpolation: 4-point Catmull-Rom, but the fraction is applied forwards from
//   x[n - int(delay)] (effective delay 2*int(d) - d: the measured saw-tooth of the delay curve).
// * Silence: Work returns false on 256-sample blocks (restarting each tick) whose input is all
//   zero once the samples of previous all-silent blocks reach
//   tail = R * (1 + ln(2^-15) / ln|feedback|) (R in samples; tail = R at 0% feedback, always cut
//   at +-100%). The counter starts "silent", so a silent first block is cut. A cut block
//   advances nothing (LFO and write pointer freeze), measured on the impulse probe.
//   Causal emulation: output is zeroed while the block has been silent so far; when input
//   arrives mid-block the LFO and write pointer catch up by the samples already skipped.
//   Block boundaries come from ma.BS (a block ends when its BS samples are done or BS changes).
import("stdfaust.lib");

freq = hslider("[0]freq", 100, 0, 4096, 1);
depth = hslider("[1]depth", 64, 0, 127, 1);
feedback = hslider("[2]feedback", 64, 0, 128, 1);
mindelay = hslider("[3]min. delay", 25, 0, 50, 1);
dryout = hslider("[4]dry out", 128, 0, 128, 1);
wetout = hslider("[5]wet out", 128, 0, 128, 1);

// masterblaster rescales raw -> 0..65535 -> slider by truncation; the error is < 0.07, so round.
raw(s) = int(s + 0.5);
fr = raw(freq);
dp = raw(depth);
md = raw(mindelay);
level(s) = (raw(s) - 64) / 64.0;
fbg = level(feedback);
dryg = level(dryout);
wetg = level(wetout);

// exact integer division of non-negative ints (float of a multiple of b is exact enough)
idiv(a, b) = int(float(a - a % b) / b + 0.5);
imod(a, m) = ((a % m) + m) % m;

ONE = 65536;
R = dp * 227569 + idiv(dp * 113, 127);              // floor(dp*441/127 * 65536)
A = md * 2890137 + idiv(md * 3, 5);                  // floor(md*44.1 * 65536)
lo = A + ONE;
hi = lo + R;
step = X1 + idiv(Y, 4410000) with {                 // floor(R * fr / 4410000) without overflow
  Rl = R % 4410;
  X = idiv(R, 4410) * fr;
  X1 = idiv(X, 1000);
  Y = (X % 1000) * 4410 + Rl * fr;
};

// tail length in samples (int); ln|fb| = 0 at +-100% means "cut at once"
tail = select2(g >= 1, select2(g == 0, int(Rs * (1 + log(2.0 ^ (-15)) / log(g))), int(Rs)), -1)
with { g = abs(fbg); Rs = float(R) / ONE; };

BIG = 1073741824;

// one ping-pong step: returns (pos, neg) where neg = 1 means moving down
step1(pos, neg) = select2(over, select2(under, p, 2 * lo - p), 2 * hi - p),
                  select2(over | under, neg, 1 - neg)
with { p = pos + (1 - 2 * neg) * step; over = p > hi; under = (p < lo) & (over == 0); };

// k ping-pong steps at once (catch-up after skipped samples); k < 256
advk(pos, neg, k) = select2(R == 0, folded_pos, odd_pos), select2(R == 0, folded_neg, odd_neg)
with {
  M = max(2 * R, 1);
  z = select2(neg, pos - lo, 2 * R - (pos - lo));
  s16 = (16 * step) % M;
  kstep = imod((k >> 4) * s16 % M + (k & 15) * step % M, M);
  zz = imod(z + kstep, M);
  folded_pos = select2(zz <= R, lo + 2 * R - zz, lo + zz);
  folded_neg = select2(zz < R, 1, 0);
  odd = k & 1;
  odd_pos = select2(odd, pos, 2 * lo - pos);
  odd_neg = select2(odd, neg, 1 - neg);
};

// Catmull-Rom weights for x[i-1], x[i], x[i+1], x[i+2]
crw(t) = (-t3 + 2 * t2 - t) / 2, (3 * t3 - 5 * t2 + 2) / 2, (-3 * t3 + 4 * t2 + t) / 2, (t3 - t2) / 2
with { t2 = t * t; t3 = t2 * t; };

// state: k, run, s0, cut, pos, neg, wp, sv (value to write), wi (index to write), y
chorus(k_, run_, s0_, cut_, pos_, neg_, wp_, sv_, wi_, y_, x) =
  k, run, s0, cut, posn, negn, wpn, svn, win, y
with {
  bs = int(ma.BS);
  first = 1 - (1 : mem);
  newblock = first | (bs != (bs : mem)) | (k_ + 1 >= bs);
  k = select2(newblock, k_ + 1, 0);
  quiet = x == 0;
  prevlen = k_ + 1;
  s0 = select2(first, select2(k == 0, s0_, select2(run_ >= prevlen, 0, min(s0_ + prevlen, BIG))), BIG);
  run = select2(quiet, 0, min(run_ + 1, BIG));
  cut = quiet & select2(k == 0, cut_, s0 >= tail);
  catch = (cut_ == 1) & (cut == 0) & (k > 0) & (first == 0);
  pc = select2(catch, pos_, (advk(pos_, neg_, k) : _, !));
  nc = select2(catch, neg_, (advk(pos_, neg_, k) : !, _));
  wpc = select2(catch, wp_, (wp_ + k) & 4095);
  G = pc >> 16;
  t = float(pc & 65535) / ONE;
  tap(j) = rwtable(4097, 0.0, wi_, sv_, (wpc - G - 1 + j) & 4095);
  wet = (tap(0), tap(1), tap(2), tap(3)), crw(t) : ro.interleave(4, 2) : par(i, 4, *) :> _;
  s = x + fbg * wet;
  active = 1 - cut;
  posn = select2(active, pos_, (step1(pc, nc) : _, !));
  negn = select2(active, neg_, (step1(pc, nc) : !, _));
  wpn = select2(active, wp_, (wpc + 1) & 4095);
  svn = select2(active, 0.0, s);
  win = select2(active, 4096, wpc);
  y = select2(active, 0.0, dryg * x + wetg * wet);
};

mono = (chorus ~ si.bus(10)) : par(i, 9, !), _;
process = _, _ : (mono <: _, _), !;
