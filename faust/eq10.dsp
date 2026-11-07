// Jon Pak EQ-10 ("10 Band Graphic EQ") — clean-room port.
//
// Derived only from the buzz-machines-oracles measurements (info.json,
// describe.json, triage.json, impulse / ESS / white-noise renders). No machine
// source was read.
//
// Findings (least-squares biquad fits of one-at-a-time noise renders divided by
// the all-default render; residuals ~1e-8 in the coefficients):
//   * Gain law (describe.json): dB = (raw - 32767) / 32767 * 12, i.e. linear
//     over the "min. gain" / "max. gain" attributes (-12 / +12 dB).
//   * The bands sit one slot below their labels (f_k = 31.25 * 2^k Hz) and the
//     controls are shifted by one:
//       "62 Hz"          -> RBJ low shelf, 31.25 Hz, S = 1
//       "125 Hz".."16 kHz" -> RBJ peaking at 62.5 .. 8000 Hz, BW = 1 octave
//                         (alpha = sin w0 * sinh(ln2/2 * BW * w0 / sin w0))
//       "31 Hz"          -> no effect at all (dead in triage too)
//     plus a fixed RBJ high shelf at 16 kHz, -12 dB, S = 1, present even at
//     defaults (the all-default render is exactly that shelf).
//   * work_returns_false: on a 256-sample block whose input is silent the
//     band cascade contributes nothing (its tail stops dead at the block
//     boundary) while the final 16 kHz shelf keeps ringing out from its own
//     state. Emulated causally: the cascade output is gated to zero while the
//     current block has been silent so far; block starts follow ma.BS.
//   * A band at exactly 0 dB is bypassed (the defaults render is bit-close to
//     the shelf alone; a single-precision chain of unity biquads is not).
//
// Sliders are in raw Buzz units (0..65534, default 32767). masterblaster rescales
// raw -> 0..65535 (truncating) -> slider range, so raw is recovered with
// floor(s * 65535 / 65534 + 0.1), which also maps an untouched default back to
// 32767.
import("stdfaust.lib");

SR = 44100.0;
BASE = 31.25;

raw(s) = min(floor(s * 65535.0 / 65534.0 + 0.1), 65534.0);
gainDb(r) = (r - 32767.0) / 32767.0 * 12.0;

s0 = hslider("[0]31 Hz", 32767, 0, 65534, 1);
r1 = hslider("[1]62 Hz", 32767, 0, 65534, 1) : raw;
r2 = hslider("[2]125 Hz", 32767, 0, 65534, 1) : raw;
r3 = hslider("[3]250 Hz", 32767, 0, 65534, 1) : raw;
r4 = hslider("[4]500 Hz", 32767, 0, 65534, 1) : raw;
r5 = hslider("[5]1 kHz", 32767, 0, 65534, 1) : raw;
r6 = hslider("[6]2 kHz", 32767, 0, 65534, 1) : raw;
r7 = hslider("[7]4 kHz", 32767, 0, 65534, 1) : raw;
r8 = hslider("[8]8 kHz", 32767, 0, 65534, 1) : raw;
r9 = hslider("[9]16 kHz", 32767, 0, 65534, 1) : raw;

// Direct form I biquad, normalised coefficients.
df1(b0, b1, b2, a1, a2) = _ <: b0 * _ + b1 * _' + b2 * _'' : + ~ feedback
with {
  feedback = _ <: -a1 * _ - a2 * _';
};

w0(f) = 2.0 * ma.PI * f / SR;

// RBJ peaking EQ, bandwidth in octaves (bilinear-warp corrected).
peakEq(f, dB) = df1(b0 / a0, b1 / a0, b2 / a0, b1 / a0, a2 / a0)
with {
  A = pow(10.0, dB / 40.0);
  w = w0(f);
  al = sin(w) * ma.sinh(log(2.0) / 2.0 * w / sin(w));
  b0 = 1.0 + al * A;
  b1 = -2.0 * cos(w);
  b2 = 1.0 - al * A;
  a0 = 1.0 + al / A;
  a2 = 1.0 - al / A;
};

// RBJ shelves with slope S = 1.
shelfParts(f, dB) = A, cos(w), 2.0 * sqrt(A) * al
with {
  A = pow(10.0, dB / 40.0);
  w = w0(f);
  al = sin(w) / 2.0 * sqrt(2.0);
};

lowShelf(f, dB) = df1(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
with {
  A = shelfParts(f, dB) : _, !, !;
  c = shelfParts(f, dB) : !, _, !;
  q = shelfParts(f, dB) : !, !, _;
  b0 = A * ((A + 1.0) - (A - 1.0) * c + q);
  b1 = 2.0 * A * ((A - 1.0) - (A + 1.0) * c);
  b2 = A * ((A + 1.0) - (A - 1.0) * c - q);
  a0 = (A + 1.0) + (A - 1.0) * c + q;
  a1 = -2.0 * ((A - 1.0) + (A + 1.0) * c);
  a2 = (A + 1.0) + (A - 1.0) * c - q;
};

highShelf(f, dB) = df1(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
with {
  A = shelfParts(f, dB) : _, !, !;
  c = shelfParts(f, dB) : !, _, !;
  q = shelfParts(f, dB) : !, !, _;
  b0 = A * ((A + 1.0) + (A - 1.0) * c + q);
  b1 = -2.0 * A * ((A - 1.0) + (A + 1.0) * c);
  b2 = A * ((A + 1.0) + (A - 1.0) * c - q);
  a0 = (A + 1.0) - (A - 1.0) * c + q;
  a1 = 2.0 * ((A - 1.0) - (A + 1.0) * c);
  a2 = (A + 1.0) - (A - 1.0) * c - q;
};

// A band at exactly 0 dB passes its input untouched.
bypassAtUnity(r, filt) = _ <: select2(r != 32767.0, _, filt);

bands = bypassAtUnity(r1, lowShelf(BASE, gainDb(r1)))
      : bypassAtUnity(r2, peakEq(BASE * 2.0, gainDb(r2)))
      : bypassAtUnity(r3, peakEq(BASE * 4.0, gainDb(r3)))
      : bypassAtUnity(r4, peakEq(BASE * 8.0, gainDb(r4)))
      : bypassAtUnity(r5, peakEq(BASE * 16.0, gainDb(r5)))
      : bypassAtUnity(r6, peakEq(BASE * 32.0, gainDb(r6)))
      : bypassAtUnity(r7, peakEq(BASE * 64.0, gainDb(r7)))
      : bypassAtUnity(r8, peakEq(BASE * 128.0, gainDb(r8)))
      : bypassAtUnity(r9, peakEq(BASE * 256.0, gainDb(r9)));

// 1 on the first sample of each host block (block length from ma.BS).
blockStart = idx == 0
with {
  idx = (\(p).(ba.if(p + 1 >= ma.BS', 0, p + 1))) ~ _;
};

// 1 while every input sample of the current block so far has been zero.
silentSoFar(x) = (\(p).((x == 0) * ba.if(blockStart, 1, p))) ~ _;

// "31 Hz" (s0) has no audible effect; it is referenced so the slider exists.
channel(x) = x : bands : *(1.0 - silentSoFar(x)) : highShelf(16000.0, -12.0) : +(s0 * ((ba.time < 0) & (ba.time > 0)));

process = channel, channel;
