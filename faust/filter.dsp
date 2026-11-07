// Jeskola Filter — clean-room Faust port.
//
// Derived only from the buzz-machines-oracles measurements (info.json, triage.json,
// impulse/ess/noise oracles); no machine source code was read.
//
// Signal path (fitted to impulse responses, residual ~1e-8 on every development config):
//   Chamberlin state-variable filter, mono, per sample:
//     low  += f1 * band
//     high  = in - low - q * band
//     band += f2 * high
//   f   = sin(pi * Cutoff / 256)            (Cutoff 128 -> f = 1)
//   q   = 1 - sin(pi * Resonance / 256)     (Resonance 0 -> q = 1, 128 -> q = 0)
//   Type 0 outputs low, 1 high, 2 band.
//   Bandwidth (bandpass only) splits the two integrator coefficients:
//     d = min(Bandwidth / 128, f, 1 - f),  f1 = f - d,  f2 = f + d.
//   Lowpass/highpass use f1 = f2 = f (Bandwidth has no effect there).
//   Cutoff 0: f = 0, so lowpass/bandpass are silent and highpass passes the input.
//
// Silent-input cut (triage: work_returns_false): the machine's output stops on any
// all-silent 256-sample block on a grid restarting each tick (5250 samples at
// 126 BPM / 4 TPB, from DSP init; masterblaster gives Faust no tempo). Emulated
// causally: output is zero while the current block has been silent so far, and a
// block that ended fully silent does not advance the filter state (rolled back to the
// block-start snapshot). Faust cannot look ahead, so a silent block prefix followed
// by signal loses the (frozen) tail for that prefix.
//
// Sliders are in raw Buzz units; masterblaster's 0..65535 rescaling truncates, so
// values are rounded with rint.

import("stdfaust.lib");

ftype  = rint(hslider("[0]Type", 0, 0, 2, 1));
cutoff = rint(hslider("[1]Cutoff", 128, 0, 128, 1));
reso   = rint(hslider("[2]Resonance", 0, 0, 128, 1));
bwidth = rint(hslider("[3]Bandwidth", 0, 0, 128, 1));

TICK = 5250;
BLOCK = 256;

f = sin(ma.PI * cutoff / 256.0);
q = 1.0 - sin(ma.PI * reso / 256.0);
d = ba.if(ftype == 2, min(bwidth / 128.0, min(f, 1.0 - f)), 0.0);
f1 = f - d;
f2 = f + d;

blockStart = ((ba.time % TICK) % BLOCK) == 0;

// Pick the output for the selected type.
selectOut(lo, hi, bp) = ba.selectn(3, ftype, lo, hi, bp);

// One sample. State in: low, band (after the previous sample), snapshot low/band
// (state at the current block's entry), silent-so-far flag. Input x, block-start st.
step(low, band, sl, sb, sil, x, st) = lowN, bandN, sl2, sb2, sil2, y
with {
  revert = st * sil;
  eL = ba.if(revert, sl, low);
  eB = ba.if(revert, sb, band);
  sl2 = ba.if(st, eL, sl);
  sb2 = ba.if(st, eB, sb);
  sil2 = ba.if(st, 1, sil) * (x == 0);
  lowN = eL + f1 * eB;
  hi = x - lowN - q * eB;
  bandN = eB + f2 * hi;
  y = selectOut(lowN, hi, bandN) * (1 - sil2);
};

filterChannel(x) = x, blockStart : (step ~ (_, _, _, _, _)) : !, !, !, !, !, _;

process = filterChannel, filterChannel;
