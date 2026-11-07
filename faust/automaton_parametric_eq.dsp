// Automaton Parametric EQ (Jon Pak) — clean-room port.
//
// Derived only from info.json / describe.json / triage.json and the oracle's measurements
// (no machine source was read). Evidence:
//  * describe.json fits (max error = display rounding), with the machine attributes at their
//    defaults (gain range 36 dB, 10..22000 Hz, freq non-linearity 6, slope 1..100,
//    bandwidth 100..20000 Hz, bandwidth non-linearity 3), x = raw / 65534:
//      gain  = -36 + 72 x                                   dB
//      freq  = 10 + 21990 (exp(6x) - 1) / (exp(6) - 1)      Hz
//      shelf = 1 + 99 x                                     (RBJ shelf slope S = 1 / shelf)
//      bw    = 100 + 19900 (exp(3x) - 1) / (exp(3) - 1)     Hz (fitted from ESS, Q = f / bw)
//  * ESS transfer functions of every development configuration fit a cascade of three RBJ
//    cookbook biquads — low shelf (LPF-*), high shelf (HPF-*), peaking band (track 0) — to
//    < 1e-4 dB.  Noise-white outputs are bit-exact against a direct-form-I model with
//    coefficients computed in double, normalised by a0 and stored as float, double
//    accumulation and float state, so this port uses DF-I in that order.
//  * work_returns_false: on 256-sample blocks (grid restarting each tick) whose input is all
//    zero the shelves are skipped (their output is 0) and the band keeps ringing; the block is
//    cut (output 0) when the band's peak |output| in the block is below 1e-20 in Buzz units
//    (1e-20 / 32768 here). Separating continued from cut impulse-probe blocks puts the
//    threshold between 1.0095e-20 and 1.0247e-20.  Emulated causally: block starts come from
//    ma.BS (a change of block length also starts a block), "silent so far" gates the shelves,
//    and the running block peak gates the output.
//  * Only track 0 is modelled (Faust has a fixed parameter list).
//  * Faust runs in single precision (float accumulation, not the DLL's double), so a 0 dB
//    section is bypassed (the DLL is bit-exact identity there) and coefficients are formed
//    as 1 + d / -2 + d to round like the DLL's double->float coefficients. Very low-frequency
//    high shelves with large cuts still differ (their b coefficients ~ A^2 (1, -2, 1) can't
//    be rounded identically in float), and the float noise floor is higher than the DLL's.
import("stdfaust.lib");

// masterblaster rescales raw -> 0..65535 (truncating) -> slider range: s*65535/65534 is then
// the integer it truncated to (= raw). floor(+0.01) also maps an untouched slider (s = init,
// where s*65535/65534 = init + 0.49999 and float rint would round up) back to its init.
raw(s) = min(65534, floor(s * 65535.0 / 65534.0 + 0.01));

lpfGain  = hslider("[0]LPF-Gain", 32767, 0, 65534, 1) : raw;
lpfFreq  = hslider("[1]LPF-Freq.", 32767, 0, 65534, 1) : raw;
lpfShelf = hslider("[2]LPF-Shelf", 0, 0, 65534, 1) : raw;
hpfGain  = hslider("[3]HPF-Gain", 32767, 0, 65534, 1) : raw;
hpfFreq  = hslider("[4]HPF-Freq.", 32767, 0, 65534, 1) : raw;
hpfShelf = hslider("[5]HPF-Shelf", 0, 0, 65534, 1) : raw;
bandGain = hslider("[6]Gain", 32767, 0, 65534, 1) : raw;
bandFreq = hslider("[7]Freq.", 32767, 0, 65534, 1) : raw;
bandBw   = hslider("[8]BWidth", 32767, 0, 65534, 1) : raw;

// ---- parameter maps (raw word -> units) ----
unit(v) = v / 65534.0;
expMap(lo, hi, k, v) = lo + (hi - lo) * (exp(k * unit(v)) - 1.0) / (exp(k) - 1.0);
gainDb(v) = 72.0 * (v - 32767.0) / 65534.0; // exact 0 dB at 7FFF
freqHz(v) = expMap(10.0, 22000.0, 6.0, v);
bwHz(v) = expMap(100.0, 20000.0, 3.0, v);
slope(v) = 1.0 + 99.0 * unit(v);

// ---- RBJ cookbook coefficients, normalised (b0, b1, b2, a1, a2) ----
// The DLL rounds double-precision coefficients to float; low-frequency responses depend on
// those last bits. To land on the same floats in single precision, coefficients close to
// (1, -2, 1) are formed as 1 + d, -2 + d, with the small d computed directly (cm = 1 - cos w
// = 2 sin^2(w/2)), so each is a single rounding of an accurate value.
amp(g) = pow(10.0, g / 40.0);
omega(f) = 2.0 * ma.PI * f / ma.SR;
oneMinusCos(f) = 2.0 * pow(sin(omega(f) / 2.0), 2);
shelfAlpha(A, f, s) = sin(omega(f)) / 2.0 * sqrt((A + 1.0 / A) * (s - 1.0) + 2.0);

// a1 / a0 = -2 + (a1 + 2 a0) / a0,  a2 / a0 = 1 + (a2 - a0) / a0
poles(a0, a1p2a0, a2ma0) = -2.0 + a1p2a0 / a0, 1.0 + a2ma0 / a0;

// low shelf: b = A (2 + p cm + sa, 2 (q cm - 2), 2 + p cm - sa); a = (2A - p cm + sa, -2 (2A - q cm), 2A - p cm - sa)
lowShelf(g, f, s) = 1.0 + p * (q * cm + sa) / a0, -2.0 + (2.0 * p * q * cm + 4.0 * cm + 2.0 * sa) / a0,
                    1.0 + (p * (q * cm - sa) - 2.0 * sa) / a0, poles(a0, 4.0 * cm + 2.0 * sa, -2.0 * sa)
with { A = amp(g); cm = oneMinusCos(f); p = A - 1.0; q = A + 1.0;
       sa = 2.0 * sqrt(A) * shelfAlpha(A, f, s); a0 = 2.0 * A - p * cm + sa; };

// high shelf: b = A (2A - p cm + sa, -2 (2A - q cm), 2A - p cm - sa); a = (2 + p cm + sa, 2 (q cm - 2), 2 + p cm - sa)
highShelf(g, f, s) = A * (2.0 * A - p * cm + sa) / a0, -2.0 * A * (2.0 * A - q * cm) / a0,
                     A * (2.0 * A - p * cm - sa) / a0, poles(a0, 4.0 * A * cm + 2.0 * sa, -2.0 * sa)
with { A = amp(g); cm = oneMinusCos(f); p = A - 1.0; q = A + 1.0;
       sa = 2.0 * sqrt(A) * shelfAlpha(A, f, s); a0 = 2.0 + p * cm + sa; };

// peaking: b = (1 + al A, -2c, 1 - al A); a = (1 + al / A, -2c, 1 - al / A); b1 = a1
peaking(g, f, bw) = 1.0 + al * (A - 1.0 / A) / a0, a1, 1.0 - al * (A + 1.0 / A) / a0, a1, a2
with { A = amp(g); cm = oneMinusCos(f); al = sin(omega(f)) * bw / (2.0 * f); a0 = 1.0 + al / A;
       a1 = poles(a0, 2.0 * (cm + al / A), -2.0 * al / A) : _, !;
       a2 = poles(a0, 2.0 * (cm + al / A), -2.0 * al / A) : !, _; };

// ---- direct form I biquad ----
df1(b0, b1, b2, a1, a2, x) = b0 * x + b1 * x' + b2 * x'' : + ~ (\(y).(0.0 - a1 * y - a2 * y'));

// At 0 dB b = a and the DLL's output is bit-identical to its input (double accumulation);
// a float DF-I would add rounding noise there, so a 0 dB section passes its input.
stage(g, coefs, x) = select2(g == 0.0, (coefs, x) : df1, x);

lowShelfStage  = stage(g, lowShelf(g, freqHz(lpfFreq), slope(lpfShelf))) with { g = gainDb(lpfGain); };
highShelfStage = stage(g, highShelf(g, freqHz(hpfFreq), slope(hpfShelf))) with { g = gainDb(hpfGain); };
bandStage      = stage(g, peaking(g, freqHz(bandFreq), bwHz(bandBw))) with { g = gainDb(bandGain); };

// ---- Buzz block grid and silent-input behaviour ----
blockStart = pos == 0
with { bs = int(ma.BS);
       pos = (\(p).(select2(bs != bs', (p + 1) % max(1, bs), 0))) ~ _; };

// any non-zero input since the start of the current block
heardInBlock(x) = (\(h).(select2(blockStart, h | (x != 0), x != 0))) ~ _;

// running peak |y| since the start of the current block
blockPeak(y) = (\(m).(select2(blockStart, max(m, abs(y)), abs(y)))) ~ _;

cutLevel = 1e-20 / 32768.0;

machine(x) = y * (heard | (blockPeak(y) >= cutLevel))
with { heard = heardInBlock(x);
       y = x : lowShelfStage : highShelfStage : *(heard) : bandStage; };

process(l, r) = machine(l) <: _, _;
