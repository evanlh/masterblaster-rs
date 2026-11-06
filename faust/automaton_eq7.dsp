// Automaton EQ-7 — 7-band graphic equalizer (clean-room port).
//
// Derived only from info.json / describe.json / triage.json and the
// buzz-machines-oracles measurements (no machine source was read):
//
// * describe.json: every band displays (raw - 32767) * 36 / 32767 dB
//   (fit error <= 0.0005 dB, i.e. display rounding); the 36 matches the
//   "Min/max gain (dB)" attribute default.
// * ESS one-at-a-time responses fit RBJ cookbook biquads to < 0.01 dB:
//     63 Hz   low shelf, f0 = 63 Hz, S = 1
//     160 Hz  peaking, f0 = 125 Hz, Q = 125/160 (bandwidth 160 Hz attribute)
//     400 Hz .. 6.3 kHz  peaking, f0 = label, Q = f0 / bandwidth = 1
//     16 kHz  high shelf, f0 = 16 kHz, S = 1
//   Cascading the seven sections reproduces every development ESS config
//   to < 0.012 dB. At 0 dB each section is exactly the identity.
// * Automation jump/ramp transients (noise-white) match direct form I with
//   coefficients switched at tick boundaries (DF-II / TDF-II are 30-100 dB
//   worse).
// * work_returns_false: output is cut on all-silent 256-sample blocks. The
//   impulse probe shows the cut sits before the 16 kHz shelf (the shelf's
//   tail continues past the cut block boundary). Emulated causally: within a
//   Faust compute block (ma.BS) the output of the first six sections is
//   zeroed while every input sample of the block so far has been zero.
//
// The machine is mono; this port is 2-in/2-out with identical, independent
// channels; the silence test looks at both inputs.
import("stdfaust.lib");

// masterblaster scales raw -> v = trunc(raw * 65535 / 65534) (so v = raw for
// raw < 65534) -> slider s = v / 65535 * 65534. Undo it: v = s * 65535 / 65534
// is an integer up to float noise. floor(+0.25) rather than rint so that an
// untouched slider at its init (32767 -> 32767.5) also maps to raw 32767.
raw(s) = min(65534, floor(s * 65535.0 / 65534.0 + 0.25));
gainDb(s) = (raw(s) - 32767.0) * 36.0 / 32767.0;
ampA(s) = pow(10.0, gainDb(s) / 40.0);

g63 = hslider("[0]63 Hz", 32767, 0, 65534, 1);
g160 = hslider("[1]160 Hz", 32767, 0, 65534, 1);
g400 = hslider("[2]400 Hz", 32767, 0, 65534, 1);
g1k = hslider("[3]1 kHz", 32767, 0, 65534, 1);
g2k5 = hslider("[4]2.5 kHz", 32767, 0, 65534, 1);
g6k3 = hslider("[5]6.3 kHz", 32767, 0, 65534, 1);
g16k = hslider("[6]16 kHz", 32767, 0, 65534, 1);

omega(f0) = 2.0 * ma.PI * f0 / ma.SR;

// Direct form I biquad (coefficients normalised by a0).
df1(b0, b1, b2, a0, a1, a2) = fir : + ~ fb
with {
  fir(x) = b0 / a0 * x + b1 / a0 * x' + b2 / a0 * x'';
  fb(y) = 0.0 - a1 / a0 * y - a2 / a0 * y';
};

// The oracle is bit-exact identity at 0 dB (default impulse response is
// exactly [1, 0, 0, ...]). Faust's float rewrite of b/a0 vs a/a0 is not, so a
// section at raw 32767 passes its input through (its DF1 state keeps running
// so leaving 0 dB is still continuous).
section(s, filt) = _ <: select2(raw(s) == 32767, filt, _);

peaking(f0, q, s) = section(s, df1(1 + al * A, -2 * c, 1 - al * A, 1 + al / A, -2 * c, 1 - al / A))
with {
  A = ampA(s);
  c = cos(omega(f0));
  al = sin(omega(f0)) / (2.0 * q);
};

// Shelf helpers, S = 1: alpha = sin(w) / 2 * sqrt(2).
shelfAl(f0) = sin(omega(f0)) / 2.0 * sqrt(2.0);

lowShelf(f0, s) = section(s, df1(A * (ap - am * c + k), 2 * A * (am - ap * c), A * (ap - am * c - k),
                      ap + am * c + k, -2 * (am + ap * c), ap + am * c - k))
with {
  A = ampA(s); ap = A + 1; am = A - 1;
  c = cos(omega(f0)); k = 2 * sqrt(A) * shelfAl(f0);
};

highShelf(f0, s) = section(s, df1(A * (ap + am * c + k), -2 * A * (am + ap * c), A * (ap + am * c - k),
                       ap - am * c + k, 2 * (am - ap * c), ap - am * c - k))
with {
  A = ampA(s); ap = A + 1; am = A - 1;
  c = cos(omega(f0)); k = 2 * sqrt(A) * shelfAl(f0);
};

mainBands = lowShelf(63, g63) : peaking(125, 125.0 / 160.0, g160)
          : peaking(400, 1, g400) : peaking(1000, 1, g1k)
          : peaking(2500, 1, g2k5) : peaking(6300, 1, g6k3);

// Position inside the current compute block: 0 .. ma.BS-1.
blockPos = next ~ _ : mem
with { next(p) = (p + 1) * ((p + 1) < ma.BS); };
// 1 while every input sample of the current block so far was zero.
silentSoFar(z) = z : (step ~ _)
with { step(prev, zz) = zz * ((blockPos == 0) | prev); };

cut(sil, y) = y * (1 - sil);

process(l, r) = sil, mainBands(l), mainBands(r)
              : (\(s, yl, yr).(cut(s, yl), cut(s, yr)))
              : highShelf(16000, g16k), highShelf(16000, g16k)
with { sil = silentSoFar((l == 0) & (r == 0)); };
