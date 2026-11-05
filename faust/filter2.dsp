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
// * Silent input: the DLL returns false from Work on a silent block, so its
//   output is zero there and its state does not advance. Buzz calls Work on
//   256-sample blocks on a grid that restarts every tick (5250 samples at the
//   oracle's 126 BPM / 4 TPB): impulse tails end at sample 512 (impulse at 441),
//   ESS tails at 132530 = tick 25 start + 5 * 256. Emulated causally: output 0
//   while the current block has been silent so far; at a block start, if the
//   previous block was entirely silent, restore the state it entered with.
//   Freeze (rather than clear) is what "Work did not run" means; no development
//   render resumes after silence, so freeze vs clear is not measurable there.
//   Faust can't look ahead, so a block whose sound starts mid-block outputs
//   zeros (not the ringing tail) for its leading silent samples.
// The machine is mono; the port filters each stereo channel independently.
import("stdfaust.lib");

// Buzz byte parameters are integers; masterblaster's 0..65535 rescaling can
// land just below one (Type 1 arrives as 0.99998), so round to the byte value.
ftype     = hslider("[0]Type", 0, 0, 2, 1) : rint : int;
cutoff    = hslider("[1]Cutoff", 128, 0, 128, 1) : rint;
resonance = hslider("[2]Resonance", 0, 0, 128, 1) : rint;

cutoff_coef(c)    = sin(ma.PI / 256.0 * pow(1.0 + c / 128.0, 7.0));
damping_coef(r)   = 1.0 - sin(ma.PI / 2.0 * r / 128.0);

// Samples per tick and per Work block (tick grid counted from DSP init).
tick_len  = 5250;
block_len = 256;

// 1 on the first sample of each Work block.
block_start = (tick_pos % block_len) == 0
with {
    tick_pos = ((+(1) : %(tick_len)) ~ _) + tick_len - 1 : %(tick_len);
};

// One Chamberlin step: (low, band) from previous (low, band) and input.
svf_core(f, q, lp, bp, x) = low, band, high
with {
    low  = lp + f * bp;
    high = x - low - q * bp;
    band = bp + f * high;
};

// Pick the Buzz output for the type: 0 = LP, 1 = HP, 2 = BP.
select_type(t, low, band, high) = ba.selectn(3, t, low, high, band);

// State entering this sample: rolled back to the saved block-start state when
// a new block begins after an entirely silent block.
entry_state(bs, sil, lp, bp, slp, sbp) = select2(restore, lp, slp), select2(restore, bp, sbp)
with { restore = bs & sil; };

// Silent-so-far flag of the current block.
silent_so_far(bs, sil, x) = (x == 0) & (bs | sil);

// Feedback (lp, bp, slp, sbp, sil), inputs (x, bs) -> feedback', output.
block_step(f, q, t, lp, bp, slp, sbp, sil, x, bs) = low, band, nslp, nsbp, nsil, out
with {
    e    = entry_state(bs, sil, lp, bp, slp, sbp);
    elp  = e : _, !;
    ebp  = e : !, _;
    nslp = select2(bs, slp, elp);
    nsbp = select2(bs, sbp, ebp);
    v    = svf_core(f, q, elp, ebp, x);
    low  = v : _, !, !;
    band = v : !, _, !;
    nsil = silent_so_far(bs, sil, x);
    out  = select2(nsil, (v : select_type(t)), 0.0);
};

filter2(x) = (block_step(cutoff_coef(cutoff), damping_coef(resonance), ftype) ~ (_, _, _, _, _))
             (x, block_start) : !, !, !, !, !, _;

process = filter2, filter2;
