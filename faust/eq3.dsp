// Jeskola EQ-3 — clean-room port (no machine source read), recovered from the
// buzz-machines-oracles measurements of variant 01d38e8d448a.
//
// Evidence (ESS one-at-a-time responses divided by the input, all gains linear):
//   out = dry·x + (gL−1)·LP(x) + (gM−1)·MID(x) + (gH−1)·HP(x)
//   * dry = Dry Thru (0/1): at the defaults the output is bit-identical to the
//     input, and Dry Thru = 0 at the defaults gives exact silence.
//   * gain dB = (v − 4096)·48/8192 (describe.json: 0 → −24.0dB, 8192 → +24.0dB).
//   * LP  = RBJ low-pass,  f = min(Low Cutoff, High Cutoff), Q = 1/√2
//     (pairs with Low Cutoff > High Cutoff fit f = High Cutoff).
//   * HP  = RBJ high-pass, f = High Cutoff, Q = 1/√2.
//   * MID = RBJ high-pass at 0.75·fL (Q = 1/√2) cascaded with an RBJ low-pass at
//     1.25·fH whose input taps are shifted one sample early: the fitted numerator
//     is [b0+b1, b2, 0] (zeros at 1, 1, −1/3, 0; poles exactly 0.75·fL / 1.25·fH
//     Butterworth), i.e. the history is updated before the taps are read.
//   * Residuals of this model vs. every development ESS job are ≈1e-5 of peak
//     (larger only for 10 Hz cutoffs, below the ESS band).
//   * work_returns_false: output is cut on every all-silent 256-sample block on
//     the per-tick block grid (impulse at 441 rings until 511, then zero). Emulated
//     causally: zero the output while the current host block has been silent so
//     far. Block starts come from ma.BS (masterblaster blocks never cross a tick).
//     The machine probably leaves its filter state untouched on silent blocks; this
//     port keeps filtering the zeros instead (unobservable in the probes).
// Dry Thru is a Buzz switch declared −1..−1; masterblaster delivers it as 0/1.
import("stdfaust.lib");

lowGain   = hslider("[0]Low Gain", 4096, 0, 8192, 1) : rint;
midGain   = hslider("[1]Mid Gain", 4096, 0, 8192, 1) : rint;
highGain  = hslider("[2]High Gain", 4096, 0, 8192, 1) : rint;
lowCut    = hslider("[3]Low Cutoff", 400, 10, 16000, 1) : rint;
highCut   = hslider("[4]High Cutoff", 5000, 10, 16000, 1) : rint;
dryThru   = hslider("[5]Dry Thru", 1, 0, 1, 1) : rint;

// ---- parameter maps ---------------------------------------------------------
gainOf(v) = pow(10.0, (v - 4096.0) * 48.0 / 8192.0 / 20.0);
fLow  = min(lowCut, highCut);
fHigh = highCut;

// ---- RBJ biquad coefficients (Q = 1/sqrt 2) ----------------------------------
omega(f) = 2.0 * ma.PI * f / ma.SR;
alpha(f) = sin(omega(f)) / (2.0 * sqrt(0.5));
a0(f) = 1.0 + alpha(f);
a1(f) = -2.0 * cos(omega(f)) / a0(f);
a2(f) = (1.0 - alpha(f)) / a0(f);
lpB(f) = (1.0 - cos(omega(f))) / (2.0 * a0(f));   // b0 = b2, b1 = 2·b0
hpB(f) = (1.0 + cos(omega(f))) / (2.0 * a0(f));   // b0 = b2, b1 = −2·b0

// direct form I: y = b0·x + b1·x' + b2·x'' − a1·y' − a2·y''
df1(b0, b1, b2, c1, c2) = fir : (+ ~ fb)
with {
  fir(x) = b0 * x + b1 * x' + b2 * x'';
  fb(y) = -c1 * y - c2 * y';
};

lowpass(f)  = df1(lpB(f), 2.0 * lpB(f), lpB(f), a1(f), a2(f));
highpass(f) = df1(hpB(f), -2.0 * hpB(f), hpB(f), a1(f), a2(f));
// low-pass with its input taps read one sample early: b0·x + b1·x + b2·x'
lowpassEarly(f) = df1(3.0 * lpB(f), lpB(f), 0.0, a1(f), a2(f));

midBand = highpass(0.75 * fLow) : lowpassEarly(1.25 * fHigh);

eq3(x) = dryThru * x
       + (gainOf(lowGain) - 1.0) * lowpass(fLow, x)
       + (gainOf(midGain) - 1.0) * midBand(x)
       + (gainOf(highGain) - 1.0) * highpass(fHigh, x);

// ---- silent-block cut (work_returns_false) ------------------------------------
sampleIndex = (+(1) ~ _) - 1 : int;                 // 0, 1, 2, … (int: no drift)
blockEnd = loop ~ _ with {                          // end of the current host block
  loop(e) = select2(sampleIndex >= e, e, sampleIndex + int(ma.BS));
};
blockStart = sampleIndex >= blockEnd';
heard(a) = loop ~ _ with {                          // block had input so far?
  loop(h) = select2(blockStart, h, 0) | (a != 0.0);
};

process(l, r) = eq3(l) * gate, eq3(r) * gate
with { gate = heard(abs(l) + abs(r)); };
