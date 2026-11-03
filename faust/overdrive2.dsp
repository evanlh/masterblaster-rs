declare name "Geonik's Overdrive 2";
declare author "masterblaster port (clean room)";

// Clean-room port of Geonik's Overdrive 2 (mono effect).
//
// No machine source was read.  Everything below was recovered from the
// machine's info.json / describe.json and from the buzz-machines-oracles
// measurements (development renders only):
//
//   * Gains.  describe.json gives Drive = raw-64 dB and Volume = (raw-128)/2 dB,
//     but the measured gains are 2^(dB/6), i.e. 6.0206 "dB" per octave
//     (Volume 0..128 spans 64.22 dB, not 64 dB).  Drive and Volume are applied
//     as 2^((Drive-64)/6) and 2^((Volume-128)/12).
//   * Shaper.  Inverting the (exactly identified) linear post filter on the
//     stepped-sine renders exposes a memoryless shaper on
//     w = x*drive/4 + A, A = 2*Assymetry/129:  w passes unchanged for |w| <= 2
//     and jumps to sign(w)*1 beyond (a "fold back to 1", not a soft clip).
//     The result is offset by A^2/4 - A, which is only visible as the DC step
//     at start-up (initial output step = A^2/4 for all tested Assymetry values).
//   * Filter.  ARX fits on the noise-pink renders at Drive 48 (linear region)
//     give, to -145 dB, a Chamberlin state-variable low-pass
//       lo += f*bp;  bp += f*(u - lo - q*bp)
//     with f = CutOff/128 and q = 1 - 0.96*Emphasis/127 (poles match for
//     every CutOff/Emphasis pair tested), followed by a DC blocker
//     (1 - z^-1)/(1 - 0.99 z^-1).
//
//   * Silence.  Tone-bursts renders go to exact zero at 256-sample block
//     boundaries counted from each tick start (tick = 5250 samples), and the
//     tail resumes from frozen state at the next onset (see "Buzz block grid").

import("stdfaust.lib");

// Parameters arrive as raw -> 0..65535 -> slider range; snap to integers.
drive    = hslider("[0]Drive", 80, 48, 128, 1) : rint;
asym     = hslider("[1]Assymetry", 44, 0, 128, 1) : rint;
cutoff   = hslider("[2]CutOff", 108, 0, 128, 1) : rint;
emphasis = hslider("[3]Emphasis", 95, 0, 128, 1) : rint;
volume   = hslider("[4]Volume", 124, 0, 128, 1) : rint;

// --- parameter -> coefficient ---------------------------------------------
driveGain = 2.0 ^ ((drive - 64) / 6.0);
volumeGain = 2.0 ^ ((volume - 128) / 12.0);
offset = 2.0 * asym / 129.0;
freq = cutoff / 128.0;
damping = 1.0 - 0.96 * emphasis / 127.0;

// --- signal path ----------------------------------------------------------
foldClip(w) = select2(w > 2.0, select2(w < -2.0, w, -1.0), 1.0);

shaper(x) = foldClip(x * driveGain * 0.25 + offset) - offset + offset * offset * 0.25;

// --- Buzz block grid ------------------------------------------------------
// The machine runs on 256-sample blocks whose grid restarts every tick
// (5250 samples at 126 BPM / 4 TPB, the oracle's tempo; Faust gets no tempo)
// and returns false -- zero output, state untouched -- for all-silent input
// blocks.  That needs a look-ahead of one block; the causal stand-in used here
// treats a block as silent while its input has stayed at exactly 0 since the
// block start (the very first sample of a render is always processed, so the
// start-up DC step lands where the original puts it).  Output stops and
// the committed filter state freezes there; the first non-zero sample resumes
// output (see the two state copies below).
tickLength = 5250;
blockLength = 256;
blockStart = (ba.time % tickLength) % blockLength == 0;
silentSoFar(x) = s letrec {
  's = (x == 0) & select2(blockStart, s, ba.time > 0);
};
active(x) = 1 - silentSoFar(x);

// --- Chamberlin SVF low-pass + DC blocker ----------------------------------
// lo[n] = lo[n-1] + f*bp[n-1];  bp[n] = bp[n-1] + f*(u[n] - lo[n] - q*bp[n-1]);
// y[n] = lo[n] - lo[n-1] + 0.99*y[n-1]  (lp holds lo[n-1]).
loNext(lo, bp) = lo + freq * bp;
bpNext(u, lo, bp) = bp + freq * (u - loNext(lo, bp) - damping * bp);
yNext(lo, bp, lp, y) = loNext(lo, bp) - lp + 0.99 * y;

// Two copies of the state: r runs every sample, restarting from the committed
// state c at each block start; c takes r's value only while the block is
// active.  So a silent block leaves c frozen, while an onset block that starts
// with zeros still has advanced through them by the time it turns active --
// as in the original.  Only the output during those leading zeros (the frozen
// tail, unknown without look-ahead) is lost.  In letrec, right-hand names are
// the previous sample's values.
from(r, c) = select2(blockStart, r, c);
hold(on, c, r) = select2(on, c, r);
filterChain(on, u) = ry letrec {
  'rlo = loNext(from(rlo, clo), from(rbp, cbp));
  'rbp = bpNext(u, from(rlo, clo), from(rbp, cbp));
  'rlp = loNext(from(rlo, clo), from(rbp, cbp));
  'ry  = yNext(from(rlo, clo), from(rbp, cbp), from(rlp, clp), from(ry, cy));
  'clo = hold(on, clo, loNext(from(rlo, clo), from(rbp, cbp)));
  'cbp = hold(on, cbp, bpNext(u, from(rlo, clo), from(rbp, cbp)));
  'clp = hold(on, clp, loNext(from(rlo, clo), from(rbp, cbp)));
  'cy  = hold(on, cy, yNext(from(rlo, clo), from(rbp, cbp), from(rlp, clp), from(ry, cy)));
};

overdrive(x) = filterChain(active(x), shaper(x)) * active(x) * volumeGain;

// Mono machine in a stereo graph: process the mono mix, feed both outputs.
monoMix(l, r) = (l + r) * 0.5;
process = monoMix : overdrive <: _, _;
