// Jeskola Filter 2 — Chamberlin state-variable filter (LP / HP / BP).
//
// No source exists for this machine. The model below was derived from the
// buzz-machines-oracles impulse renders (development set) and reproduces them
// to float32 precision (max relative error ~1e-7 on every dev configuration):
//
// * Structure: Chamberlin SVF, one pass per sample, low updated first:
//     low  += f * band
//     high  = in - low - q * band
//     band += f * high
//   LP output is `low` (one-sample latency, numerator f^2 z^-1), HP is `high`
//   (numerator (1 - z^-1)^2), BP is `band` (numerator f (1 - z^-1)). Fitted
//   biquad coefficients match these numerators exactly.
// * Cutoff 0..128 -> f = sin(pi/256 * (1 + Cutoff/128)^7). Measured f at
//   cutoff 0 is pi/256 and at 128 is exactly 1 (f = 1, q = 1 makes LP a pure
//   one-sample delay, HP/BP the matching complements: the "fully open" filter).
//   asin(f) / (pi/256) = (1 + x)^7 holds at every measured cutoff.
// * Resonance 0..128 -> q = 1 - sin(pi/2 * Resonance/128); q = 0 at 128
//   (undamped, rings indefinitely, as the DLL does).
// * Type 0 = LP, 1 = HP, 2 = BP (Buzz has no notch).
//
// Known difference: the DLL returns false from Work on silent input, so its
// output stops when the input does; this port keeps ringing.
// The machine is mono; the port filters each stereo channel independently.
import("stdfaust.lib");

// Buzz byte parameters are integers; masterblaster's 0..65535 rescaling can
// land just below one (Type 1 arrives as 0.99998), so round to the byte value.
ftype     = hslider("[0]Type", 0, 0, 2, 1) : rint : int;
cutoff    = hslider("[1]Cutoff", 128, 0, 128, 1) : rint;
resonance = hslider("[2]Resonance", 0, 0, 128, 1) : rint;

cutoff_coef(c)    = sin(ma.PI / 256.0 * pow(1.0 + c / 128.0, 7.0));
damping_coef(r)   = 1.0 - sin(ma.PI / 2.0 * r / 128.0);

// One Chamberlin step: previous (low, band) and input -> (low, band, high).
svf_step(f, q, lp, bp, x) = low, band, high
with {
    low  = lp + f * bp;
    high = x - low - q * bp;
    band = bp + f * high;
};

// (low, band, high) outputs of the SVF.
svf(f, q) = svf_step(f, q) ~ (_, _);

// Pick the Buzz output for the type: 0 = LP, 1 = HP, 2 = BP.
select_type(t, low, band, high) = ba.selectn(3, t, low, high, band);

filter2 = svf(cutoff_coef(cutoff), damping_coef(resonance)) : select_type(ftype);

process = filter2, filter2;
