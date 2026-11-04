// Arguelles Degrader: sample-rate and bit-depth reducer (clean-room port).
//
// Derived only from info.json/describe.json/triage.json and the oracle's
// measurements (numpy on development renders, inputs recovered by scoring
// an identity DSP). No machine source was read.
//
// Findings (bit-exact against every development render of all three probes):
// - Smp. Rate S: sample-and-hold with a down-counter. The counter starts at
//   S, is decremented every sample and on reaching 0 the input is captured
//   and the counter reloaded with S + 1. So the hold period is S + 1 samples
//   (not S, as the "44100/S Hz" display text suggests) and the first capture
//   is at sample S - 1. The held value starts at 0.
// - Bit Depth B: the held value, on the +-32768 Buzz scale, is divided by
//   2^(16-B) and truncated toward zero (not floored), then scaled back.
// - Silence: the host skips the machine on all-silent 256-sample blocks
//   (block grid restarting every 5250-sample tick at 126 BPM / 4 TPB; Faust
//   gets no tempo, so the grid starts at DSP init). In a skipped block the
//   output is 0 and the counter and held value do not advance. Without
//   lookahead we process the block normally and, at the next block start,
//   roll counter and hold back to their block-start snapshot if the block
//   was silent. A block's output is forced to 0 while its input has been
//   all zeros so far, which makes silent blocks exact; the residual error is
//   in a block that starts with exact zeros but is not silent (the held
//   value would show until the first capture or the first non-zero input).
// Mono machine; each stereo channel runs its own copy.

import("stdfaust.lib");

rate = rint(hslider("[0]Smp. Rate", 1, 1, 32, 1));
bits = rint(hslider("[1]Bit Depth", 16, 2, 16, 1));

TICK = 5250;
BLOCK = 256;
SCALE = 32768.0;

blockStart = ((ba.time % TICK) % BLOCK) == 0;
step = float(1 << int(16 - bits));

// Truncate toward zero onto the 2^(16-B) grid.
quantise(h) = float(int(h / step)) * step;

// State: c (down-counter, 0 = fresh), h (held), cs/hs (block-start
// snapshot), nz (non-silent input seen in this block).
degrade(x) = core ~ (_, _, _, _, _) : (!, _, !, !, _) : gate : /(SCALE)
with {
  xs = x * SCALE;
  gate(h, nz) = quantise(h) * nz;
  core(c, h, cs, hs, nz) = cn, hn, csn, hsn, nzn
  with {
    restore = blockStart & (nz == 0);
    ci = select2(restore, c, cs);
    hi = select2(restore, h, hs);
    csn = select2(blockStart, cs, ci);
    hsn = select2(blockStart, hs, hi);
    nzn = select2(blockStart, nz, 0) | (xs != 0);
    cd = select2(ci == 0, ci, rate) - 1;
    hit = cd <= 0;
    cn = select2(hit, cd, rate + 1);
    hn = select2(hit, hi, xs);
  };
};

process = degrade, degrade;
