// Rout EQ-10 (Mathieu Routhier) — 10-band graphic EQ, clean-room port.
//
// Derived only from the oracle measurements (no machine source was read):
// * Output is exactly linear in the band gains (least squares over all 324
//   development ESS jobs, residual 3e-15): y = sum_k (v_k / 127) * BPF_k(x).
//   describe.json agrees: value v displays as round(v / 1.27) "%", 127 = 100 %.
//   There is no dry path; all bands at 0 give silence.
// * Each band response fits an RBJ constant-0 dB-peak bandpass biquad
//     b = al/(1+al) * (1, 0, -1),  a = (1, -2 cos(w0)/(1+al), (1-al)/(1+al))
//   to ~1e-7 rms, with w0 = 2 pi fc / fs and al = tan(pi B / fs): a band
//   specified by its edges [lo, hi], fc = (lo + hi) / 2, B = hi - lo.
//   Fitted (fc, B) in Hz: (23, 46) (62, 46) (125, 94) (250, 188) (500, 375)
//   (1000, 750) (2000, 1500) (4000, 3000) (8000, 6000) (17025, 10050).
//   The first band is [0, 46] Hz and the last [12000, 22050] Hz (Nyquist).
//   Residual deviations at the lowest bands match single-precision coefficients.
// * Mono machine: both channels run the same filter bank.
// * Silent-input cut (Work returns false): the output is zero for every
//   all-silent input block. Blocks are 256 samples on a grid that restarts at
//   each 5250-sample tick (126 BPM, 4 TPB at 44.1 kHz; 20 blocks of 256 + one
//   of 130), counted from DSP init. Evidence: the ESS input ends at sample
//   132298 and every development output stops at 132529 = 25*5250 + 5*256 - 1
//   (a plain 256 grid would stop at 132351); impulse outputs stop at 511.
//   Emulated causally: output is muted while the current block has been
//   silent so far (exact unless a block starts silent, then gets input).
//   Whether filter state is frozen or cleared across silent blocks is not
//   observable in any probe (input never resumes after silence), so the
//   filters simply keep running on the zero input. masterblaster gives Faust
//   no tempo, so the 5250-sample tick assumes 126 BPM / 4 TPB.
import("stdfaust.lib");

// Raw byte 0..254 (127 = unity); rint undoes masterblaster's 0..65535 truncation.
gain(v) = rint(v) / 127.0;
gains = gain(hslider("[0]31 Hz", 127, 0, 254, 1)),
        gain(hslider("[1]62 Hz", 127, 0, 254, 1)),
        gain(hslider("[2]125 Hz", 127, 0, 254, 1)),
        gain(hslider("[3]250 Hz", 127, 0, 254, 1)),
        gain(hslider("[4]500 Hz", 127, 0, 254, 1)),
        gain(hslider("[5]1 kHz", 127, 0, 254, 1)),
        gain(hslider("[6]2 kHz", 127, 0, 254, 1)),
        gain(hslider("[7]4 kHz", 127, 0, 254, 1)),
        gain(hslider("[8]8 kHz", 127, 0, 254, 1)),
        gain(hslider("[9]16 kHz", 127, 0, 254, 1));

centers = (23, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 17025);
widths = (46, 46, 94, 188, 375, 750, 1500, 3000, 6000, 10050);

// RBJ constant-peak bandpass from centre fc and bandwidth bw (both Hz).
bandpass(fc, bw) = fi.tf2(b0, 0.0, 0.0 - b0, a1, a2)
with {
    al = tan(ma.PI * bw / ma.SR);
    a0 = 1.0 + al;
    b0 = al / a0;
    a1 = -2.0 * cos(2.0 * ma.PI * fc / ma.SR) / a0;
    a2 = (1.0 - al) / a0;
};

band(k) = bandpass(ba.take(k + 1, centers), ba.take(k + 1, widths)) * ba.take(k + 1, gains);

eq = _ <: par(k, 10, band(k)) :> _;

// Position inside the current tick, 0 at DSP init.
tick_len = 5250;
tick_pos = (+(1) : %(tick_len)) ~ _ : +(tick_len - 1) : %(tick_len);
block_start = (tick_pos % 256) == 0;

// 1 once the current block has had a non-zero input sample.
heard(x) = (x != 0) : (max ~ *(1 - block_start));

gated_eq(x) = eq(x) * heard(x);

process = gated_eq, gated_eq;
