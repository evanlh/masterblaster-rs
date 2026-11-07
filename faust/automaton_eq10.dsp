// Automaton EQ-10 ("10 Band Graphic EQ", Jon Pak) — clean-room port.
//
// Derived only from info.json/describe.json/triage.json and oracle measurements
// (no machine source read). Evidence:
//  * describe.json: gain dB = (raw - 32767) * 12 / 32767 (−12 … +12 dB, linear in raw).
//  * ESS one-at-a-time responses, isolated by dividing by the default response and fitted
//    with a 2nd-order Levy fit: every band is an RBJ biquad, cascaded (gains add in dB).
//    The bands are shifted by one slot relative to their labels:
//      "62 Hz"  -> low shelf 31.25 Hz, S = 1
//      "125 Hz" … "16 kHz" -> RBJ peaking at 62.5 … 8000 Hz, BW = 1 octave (digital BW form)
//      "31 Hz"  -> no effect at all (triage lists it as dead)
//    plus a fixed RBJ high shelf at 16 kHz, S = 1, −12 dB, present at every setting
//    (the default response is −6.00 dB at 16 kHz, −11.83 dB at 20 kHz).
//  * Filter structure: direct form I. Simulating the automation-jump noise jobs with DF-I,
//    DF-II and transposed DF-II, only DF-I reproduces the transient at the coefficient change
//    (residual 4e-4 vs 5e-2); parameters take effect at the tick start, no smoothing.
//  * Silent input (triage work_returns_false): on 256-sample blocks on a grid restarting
//    every tick (5250 samples at 126 BPM / 4 TPB, counted from DSP init), the output of the
//    band cascade stops on an all-silent block, but the fixed high shelf keeps ringing out
//    its own state (impulse and ESS tails end ~120 samples into the silent block). Emulated
//    causally: the cascade output is muted while the current block has been silent so far.
import("stdfaust.lib");

SR = 44100.0;
TICK = 5250;
BLOCK = 256;

// Sliders in raw Buzz word units (design 005). masterblaster sends u = trunc(raw * 65535/65534)
// and sets the slider to u * 65534/65535; undo that exactly (u = 65535 only for raw 65534).
word(s) = min(65534.0, rint(s * 65535.0 / 65534.0));
w0 = word(hslider("[0]31 Hz", 32767, 0, 65534, 1));
w1 = word(hslider("[1]62 Hz", 32767, 0, 65534, 1));
w2 = word(hslider("[2]125 Hz", 32767, 0, 65534, 1));
w3 = word(hslider("[3]250 Hz", 32767, 0, 65534, 1));
w4 = word(hslider("[4]500 Hz", 32767, 0, 65534, 1));
w5 = word(hslider("[5]1 kHz", 32767, 0, 65534, 1));
w6 = word(hslider("[6]2 kHz", 32767, 0, 65534, 1));
w7 = word(hslider("[7]4 kHz", 32767, 0, 65534, 1));
w8 = word(hslider("[8]8 kHz", 32767, 0, 65534, 1));
w9 = word(hslider("[9]16 kHz", 32767, 0, 65534, 1));

// raw word -> gain in dB (describe.json), then RBJ amplitude A = 10^(dB/40).
db(w) = (w - 32767.0) * 12.0 / 32767.0;
amp(g) = pow(10.0, g / 40.0);

// RBJ biquads, normalised by a0. cos(w) = 1 - 2 sin^2(w/2) keeps low bands accurate in float.
omega(fc) = 2.0 * ma.PI * fc / SR;
cosw(fc) = 1.0 - 2.0 * pow(sin(omega(fc) / 2.0), 2.0);

// Peaking EQ, bandwidth in octaves (digital form: alpha = sin w * sinh(ln2/2 * BW * w / sin w)).
peak(fc, g) = fi.tf21(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
with {
  A = amp(g); w = omega(fc); s = sin(w); c = cosw(fc);
  al = s * ma.sinh(log(2.0) / 2.0 * w / s);
  b0 = 1.0 + al * A; b1 = -2.0 * c; b2 = 1.0 - al * A;
  a0 = 1.0 + al / A; a1 = -2.0 * c; a2 = 1.0 - al / A;
};

// Shelves with slope S = 1 (alpha = sin w / sqrt 2); hi = 1 for the high shelf.
shelf(hi, fc, g) = fi.tf21(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
with {
  A = amp(g); w = omega(fc); c = cosw(fc) * sg; sg = 1.0 - 2.0 * hi;
  sq = 2.0 * sqrt(A) * sin(w) / sqrt(2.0);
  b0 = A * ((A + 1.0) - (A - 1.0) * c + sq);
  b1 = 2.0 * A * ((A - 1.0) - (A + 1.0) * c) * sg;
  b2 = A * ((A + 1.0) - (A - 1.0) * c - sq);
  a0 = (A + 1.0) + (A - 1.0) * c + sq;
  a1 = -2.0 * ((A - 1.0) + (A + 1.0) * c) * sg;
  a2 = (A + 1.0) + (A - 1.0) * c - sq;
};

// Band cascade: labels are shifted one slot down ("31 Hz" reaches no filter).
bands = shelf(0, 31.25, db(w1))
      : peak(62.5, db(w2)) : peak(125.0, db(w3)) : peak(250.0, db(w4))
      : peak(500.0, db(w5)) : peak(1000.0, db(w6)) : peak(2000.0, db(w7))
      : peak(4000.0, db(w8)) : peak(8000.0, db(w9))
      : attach(_, w0);  // keep the dead "31 Hz" slider in the UI

// Fixed output stage: high shelf 16 kHz, -12 dB.
outshelf = shelf(1, 16000.0, -12.0);

// Sample counter from DSP init and the 256-sample block grid restarting every tick.
pos = (+(1) ~ _) - 1 : %(TICK);
blockstart = (pos % BLOCK) == 0;

// 1 while every input sample of the current block so far was exactly zero.
silent(x) = (x == 0) : (s ~ _) with { s(prev, z) = z * (blockstart + (1 - blockstart) * prev > 0); };

channel(x) = bands(x) * (1 - silent(x)) : outshelf;

process = channel, channel;
