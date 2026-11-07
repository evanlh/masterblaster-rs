// Jeskola NiNjA dElaY — filtered feedback delay (clean-room port).
//
// Derived only from info.json and the buzz-machines-oracles measurements
// (impulse / burst-then-silence / noise-pink development jobs, numpy fits);
// no machine source was read.
//
// Signal path (track 0; matches the oracle to float precision):
//   line[n] = x[n] + fb * w[n]
//   w[n]    = F( line[n - D] )            biquad F inside the loop
//   y[n]    = dry * x[n] + wet * w[n]
// - D: exact integer tap, no interpolation. Units: tick = L*5250 (126 BPM,
//   4 TPB), ms = floor(L*44.1), samples = L, 1/256 tick = floor(L*5250/256);
//   capped at the Max Delay attribute (default 1000 ms = 44100 samples).
// - fb = (raw-64)/64, wet = raw/128, dry thru = 0/1 (unity, no delay).
// - F: RBJ cookbook biquads (fitted coefficient-exact from impulse echoes):
//   LP/HP with Q = 1/sqrt2; BP (0 dB peak) / notch with alpha = tan(pi*bw/sr).
// - Silent input (Work returns false): Buzz flags silence per block (256
//   samples, grid restarting each tick). With run = silent samples in the
//   whole silent blocks before the current one and
//   T = D*(1 + 15/log2(1/|fb|)) (T = D at fb 0 or +-1):
//   a silent block with run >= T clears the delay line (the filter keeps
//   ringing, feedback keeps writing); the next such block outputs nothing.
//   (Seen by inverse-filtering the oracle: the filter input drops to zero at
//   the gate block start and fb*w reappears exactly D samples later.) Block starts come
//   from the host block size (ma.BS). Causal approximation: a block counts as
//   silent while it has been silent so far.
// Parameters arrive raw -> 0..65535 -> slider range by truncation; raw values
// are recovered by rounding (the error is always below half a unit).
import("stdfaust.lib");

SR = 44100.0;
TICK = 5250;
MAXD = 44100;

raw(s) = int(s + 0.5);

dryThru = hslider("[0]Dry thru", 1, 0, 1, 1) : raw;
lenRaw = hslider("[1]Length", 3, 1, 32768, 1) : raw;
unit = hslider("[2]Length unit", 0, 0, 3, 1) : raw;
fbRaw = hslider("[3]Feedback", 112, 0, 128, 1) : raw;
ftype = hslider("[4]f. type", 1, 0, 3, 1) : raw;
cutoff = hslider("[5]f. cutoff", 550, 10, 20000, 1) : raw;
bandwidth = hslider("[6]f. bandwidth", 100, 1, 10000, 1) : raw;
wetRaw = hslider("[7]Wet out", 96, 0, 128, 1) : raw;

// ---- parameter -> coefficient formulas ----
msLen(l) = int(floor(float(l * 441) / 10.0 + 0.01));
tickLen(l) = min(l, 9) * TICK;              // 9 ticks already exceed MAXD
subTickLen(l) = (min(l, 4096) * TICK) >> 8;
delayLen(l, u) = ba.selectn(4, u, tickLen(l), msLen(l), l, subTickLen(l)) : min(MAXD) : max(1);
D = delayLen(lenRaw, unit);
fb = (fbRaw - 64) / 64.0;
wet = wetRaw / 128.0;

tailLen(d, f) = select2((f == 0) | (f >= 1), d * (1.0 + 15.0 * log(2.0) / log(1.0 / max(f, 1e-9))), d);
T = tailLen(D, abs(fb));

// RBJ biquad, normalised by a0. The machine evidently computes coefficients
// in double and filters in float (its marginal fb=+-1 / 10 Hz loops only
// track with double-rounded coefficients), so the formulas below avoid float
// cancellation: 1 -+ cos(w) via sin^2(w/2), a1 as -2 + small term.
w0 = 2.0 * ma.PI * cutoff / SR;
s2 = sin(w0 / 2.0) * sin(w0 / 2.0);
alpha = select2(ftype >= 2, sin(w0) / (2.0 * sqrt(0.5)), tan(ma.PI * bandwidth / SR));
a0 = 1.0 + alpha;
a1 = 2.0 * (alpha + 2.0 * s2) / a0 - 2.0;
a2 = 1.0 - 2.0 * alpha / a0;
bLP = s2 / a0, 2.0 * s2 / a0, s2 / a0;
bHP = (1.0 - s2) / a0, -2.0 * (1.0 - s2) / a0, (1.0 - s2) / a0;
bBP = alpha / a0, 0.0, 0.0 - alpha / a0;
bBR = 1.0 / a0, a1, 1.0 / a0;
pick(k) = ba.selectn(4, ftype, ba.take(k, (bLP)), ba.take(k, (bHP)), ba.take(k, (bBP)), ba.take(k, (bBR)));

// direct form I in float
df1(b0, b1, b2) = fi.fir((b0, b1, b2)) : (+ ~ (\(yp).(0.0 - a1 * yp - a2 * yp')));
biquad = df1(pick(1), pick(2), pick(3));

// ---- block grid and silence bookkeeping ----
// q = position in block + 1; a block starts when the previous one filled ma.BS.
blockPos = (\(qp).(select2(qp >= ma.BS', qp, 0) + 1)) ~ _;
blockStart = blockPos == 1;

// nonzero input seen in the current block so far
blockHeard(x) = (\(h).(select2(blockStart, h, 0) | (x != 0))) ~ _;

// silent samples in whole silent blocks before the current block
silentRun(x) = (\(r).(select2(blockStart, r, select2(blockHeard(x)', r + ma.BS', 0)))) ~ _;

// gate: a silent block with run >= T clears the delay line (the filter keeps
// ringing and feedback writes continue); the next such block is cut.
readStopped(x) = (silentRun(x) >= T) & (blockHeard(x) == 0);
cutNow(x) = (\(c).(select2(blockStart, c, readStopped(x)') & (readStopped(x)))) ~ _;

// clearing = bumping an epoch; a tap is valid only if written in this epoch
epoch(x) = (+(blockStart & readStopped(x))) ~ _;
tapValid(x, d) = (epoch(x) @ d) == epoch(x);

// ---- delay loop ----
delayed(s, d) = s @ d;
tap(x, wp) = tapValid(x, D) * (delayed(x, D) + fb * delayed(wp, D - 1));
wetPath(x) = (\(wp).(biquad(tap(x, wp)))) ~ _;

mono(x) = select2(cutNow(x), dryThru * x + wet * wetPath(x), 0.0);

process(l, r) = mono(0.5 * (l + r)) <: _, _;
