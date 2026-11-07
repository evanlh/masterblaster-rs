// Jeskola Flanger: clean-room port (no machine source read).
// Model recovered only from info.json/describe.json and the buzz-machines-oracles
// measurements (impulse / white-noise / ESS responses, per-sample delay tracking).
//
// Signal path (mono machine, run independently on L and R):
//   wet  = cubic Hermite (Catmull-Rom) read from a 1024-sample ring buffer
//   buf[w] = in + fb * wet          (buffer written AFTER the read, so buf[w] is stale)
//   out  = dry * in + wet_out * wet
//   gains: (raw - 64) / 64 for feedback, dry out and wet out (describe.json)
//
// Delay curve d(n) (in samples), measured by per-sample inversion of the noise probe:
//   lo = mindelay * 4.41            (0.1 ms units)
//   hi = lo + depth * 441 / 127     (depth 127 = 10 ms swing)
//   step = (hi - lo) * (freq / 100 Hz) / 44100   (so one up+down sweep takes 2/f s),
//          truncated to 1/65536 sample (measured slope ratios 0.99945, 0.99921, 0.99975,
//          0.99987 for (depth,freq) = (127,100), (64,100), (64,1024), (127,1024) match
//          floor(step*65536)/65536 exactly) -> LFO kept as 16.16 integers
//   start: d = 2*lo + 1 moving downwards (measured: freq 0 holds d = 2*lo + 1)
//   each sample: reflect at the bounds (d = 2*bound - d, flip direction), read, d += step
//   With depth 0 the reflection makes d alternate between 2lo+1 and -1 every sample,
//   which reproduces the oracle's odd/even-sample behaviour exactly.
// Read quirk: i = floor(d), t = d - i (16.16 fixed point), centre tap at w - i - 1, i.e. the fraction is
// applied in the "wrong" direction (effective delay i + 1 - t), as the oracle shows.
//
// Silent input (work_returns_false): blocks of the host (ma.BS, 256 restarting each tick)
// whose input is entirely zero produce zero output and do not advance the LFO or the
// write pointer, once the silence has lasted longer than a feedback-dependent hold
// (see `hold` below). Causal emulation: output 0 while the block has been silent so far,
// and at the next block start roll the LFO / write index back to the saved state.
// Known residual: mid-song parameter changes (automation jobs) — how the original
// re-anchors d when lo/hi/step change is not measurable from tick-0 probes; here d
// simply continues and reflects at the new bounds.
import("stdfaust.lib");

N = 1024;

// raw Buzz value from a slider rescaled raw -> 0..65535 -> min..max by truncation
raw(R, s) = ceil(rint(s * 65535.0 / R) * R / 65535.0 - 0.002);

freq  = raw(4096, hslider("[0]freq", 100, 0, 4096, 1));
depth = raw(127,  hslider("[1]depth", 64, 0, 127, 1));
fbr   = raw(128,  hslider("[2]feedback", 64, 0, 128, 1));
mind  = raw(50,   hslider("[3]min. delay", 0, 0, 50, 1));
dryr  = raw(128,  hslider("[4]dry out", 128, 0, 128, 1));
wetr  = raw(128,  hslider("[5]wet out", 0, 0, 128, 1));

gain(v) = (v - 64) / 64;
// LFO in 16.16 fixed point: the measured per-sample step is exactly
// floor(step * 65536) / 65536 (e.g. 655/65536 instead of 0.01 at 1 Hz, depth 127)
FX = 65536;
// exact integer arithmetic (float32 would be off by ~1 unit at 2^24)
fxdiv(q, den) = int(q / den) * FX + int((q % den) * FX / den);  // floor(q * FX / den)
lo = fxdiv(int(mind) * 441, 100);
hi = lo + fxdiv(int(depth) * 441, 127);
stp = int(int(depth) * int(freq) * 4096 / 79375);  // floor(A * f/100 / 44100 * FX)

// reflect a delay value at the bounds -> (d, sign)
reflect(d, sg) = select2(d > hi, select2(d < lo, d, 2 * lo - d), 2 * hi - d),
                 select2((d > hi) | (d < lo), sg, 0 - sg);

// silent-input hold (measured from where impulse / ESS tails end): after the input
// goes silent the machine keeps processing whole blocks while the silence so far is
// < H samples, H = 16 * A / log2(1/|fb|) with log2(1/|fb|) capped at 16 (time for
// the feedback tail to fall 96 dB over the depth span A; fb = 0 gives H = A).
// |fb| = 1 gives no hold at all (measured; presumably a divide-by-zero overflow).
afb = abs(gain(fbr));
hold = select2(afb >= 1, 16.0 * depth * 441.0 / 127.0
               / (0 - log(max(afb, 1.0 / 65536.0)) / log(2.0)), 0);

// block / LFO state machine; q = current input sample is silent
// state: r sil d sg w ds sgs ws ini c bl gb | outputs mute dr wb
lfo(q) = upd ~ si.bus(12) : (si.block(12), _, _, _)
with {
  upd(r, sil, d, sg, w, ds, sgs, ws, ini, c, bl, gb) =
      r1, sil1, dr + sgr * stp, sgr, (wb + 1) % N, ds1, sgs1, ws1, 1, c1, bl1, gb1,
      sil1 * gb1, dr, wb
  with {
    start = r <= 1;
    r1 = select2(start, r - 1, ma.BS);
    sil1 = select2(start, sil, 1) * q;
    psil = (sil > 0) & (ini > 0);              // previous block was entirely silent
    c1 = select2(start, c, select2(psil, 0, c + bl) + (1 - ini) * 1e9);
    bl1 = select2(start, bl, ma.BS);
    gb1 = select2(start, gb, c1 >= hold);      // block not processed if it stays silent
    rb = start & psil & (gb > 0);
    db = select2(ini > 0, 2 * lo + FX, select2(rb, d, ds));
    sgb = select2(ini > 0, -1, select2(rb, sg, sgs));
    wb = select2(ini > 0, 0, select2(rb, w, ws));
    ds1 = select2(start, ds, db);
    sgs1 = select2(start, sgs, sgb);
    ws1 = select2(start, ws, wb);
    dr = reflect(db, sgb) : (_, !);
    sgr = reflect(db, sgb) : (!, _);
  };
};

hermite(p0, p1, p2, p3, t) =
  ((((0 - p0 + 3 * p1 - 3 * p2 + p3) * 0.5 * t) + (p0 - 2.5 * p1 + 2 * p2 - 0.5 * p3)) * t
   + 0.5 * (p2 - p0)) * t + p1;

// one channel: feedback loop carries the value written to the buffer
chan(mute, d, w, x) = (loop ~ _) : (!, _)
with {
  i = d >> 16;
  t = float(d & 65535) / 65536.0;
  c = w - i - 1;
  ix(k) = int((c + k + 2 * N) % N);
  tab(v, k) = rwtable(N, 0.0, int(w'), v, ix(k));   // write previous sample's value
  loop(v) = x + gain(fbr) * y, select2(mute > 0, gain(dryr) * x + gain(wetr) * y, 0)
  with {
    y = hermite(tab(v, -1), tab(v, 0), tab(v, 1), tab(v, 2), t);
  };
};

process(xl, xr) = lfo((abs(xl) + abs(xr)) == 0)
  : \(mute, d, w).(chan(mute, d, w, xl), chan(mute, d, w, xr));
